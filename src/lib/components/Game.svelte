<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { Channel } from "@tauri-apps/api/core";
    import { playHit, playWall, playMyGoal, playOpponentGoal,
             playNearMiss, playCountdownTick, playCountdownGo,
             playWin, playLose, initAudio } from "$lib/audio";
    import { InterstitialAd } from "tauri-plugin-admob-api";

    let {
        isHost,
        isSinglePlayer = false,
        useUdp = false,
        onBack,
    } = $props<{
        isHost: boolean;
        isSinglePlayer?: boolean;
        useUdp?: boolean;
        onBack?: () => void;
    }>();

    const WINNING_SCORE = 6;

    const AD_UNIT_ID = "ca-app-pub-7224112237798955/1828655479";
    // Preloaded interstitial — ready before game ends
    let preloadedAd: InstanceType<typeof InterstitialAd> | null = null;

    async function preloadAd() {
        try {
            const ad = new InterstitialAd({ adUnitId: AD_UNIT_ID });
            await ad.load();
            preloadedAd = ad;
        } catch {
            preloadedAd = null;
        }
    }

    async function showAd() {
        const ad = preloadedAd;
        preloadedAd = null;
        // Start preloading the next one immediately
        preloadAd();
        if (!ad) return;
        try {
            await ad.show();
        } catch {
            // Ad failure is non-fatal — game-over overlay always shows
        }
    }

    // ── Constants (render only) ───────────────────────────────────────────────
    const TW = 360,
        TH = 640;
    const PR = 20,
        PAR = 27;
    const GOAL_W = 110;
    const CR = 42;
    const MAX_SPEED = 990;
    const GX = (TW - GOAL_W) / 2;

    // ── Render state (populated by Rust via Channel) ──────────────────────────
    interface RS {
        puck: [number, number];
        puck_speed: number;
        host_paddle: [number, number];
        client_paddle: [number, number];
        score: [number, number];
        wall_flash: number;
        goal_flash: number;
        score_flash: [number, number];
        hit: number;
        wall_hit: number;
        goal_scored: number;
        countdown: number;
    }

    let rs: RS = {
        puck: [TW / 2, TH / 2],
        puck_speed: 0,
        host_paddle: [TW / 2, TH - 120],
        client_paddle: [TW / 2, 120],
        score: [0, 0],
        wall_flash: 0,
        goal_flash: 0,
        score_flash: [0, 0],
        hit: 0,
        wall_hit: 0,
        goal_scored: 0,
        countdown: 3,
    };

    let gameOver = $state(false);
    let iWon = $state(false);
    let muted = $state(false);

    // Pending pointer position — flushed to Rust once per rAF frame
    let pendingPtr: [number, number] | null = null;

    // Trail maintained locally (JS side) from puck position updates
    let trail: { x: number; y: number; age: number }[] = [];
    let lastPuckX = TW / 2,
        lastPuckY = TH / 2;
    let lastTime = 0;

    // ── Audio tracking ────────────────────────────────────────────────────────
    let prevScore: [number, number] = [0, 0];
    let prevNumLit = -1;
    let prevCountdownActive = true; // starts with countdown=3
    let puckNearMyGoal = false;     // near-miss detection
    let nearMissCooldown = 0;       // ms since last near-miss, prevent rapid firing

    function handleAudio(state: RS) {
        const myIdx = isHost ? 0 : 1;
        const opIdx = isHost ? 1 : 0;

        // Paddle hit — speed-modulated crunch
        if (state.hit) {
            if (!muted) playHit(state.puck_speed);
            navigator.vibrate?.(12);
        }
        // Wall bounce
        if (state.wall_hit) {
            if (!muted) playWall();
            navigator.vibrate?.(6);
        }
        // Goal — distinguish mine vs opponent's
        if (state.goal_scored) {
            const iScored = state.score[myIdx] > prevScore[myIdx];
            if (!muted) iScored ? playMyGoal() : playOpponentGoal();
            navigator.vibrate?.(iScored ? [50, 30, 80] : [40, 30, 40]);
        }
        prevScore = [state.score[0], state.score[1]];

        // Near miss — puck was inside the goal gap and close to my goal line, but didn't score.
        // Only triggers if puck was in the goal gap (x within posts) and within 12px of the line.
        const inGap = state.puck[0] > GX && state.puck[0] < GX + GOAL_W;
        const myGoalY = isHost ? TH - 12 : 12;
        const nearNow = inGap && (isHost ? state.puck[1] > myGoalY : state.puck[1] < myGoalY);
        nearMissCooldown = Math.max(0, nearMissCooldown - 33); // ~1 tick at 30Hz
        if (puckNearMyGoal && !nearNow && !state.goal_scored && nearMissCooldown === 0) {
            if (!muted) playNearMiss();
            nearMissCooldown = 2000; // 2-second cooldown
        }
        puckNearMyGoal = nearNow;

        // F1 countdown lights — tick per light, GO when lights out
        if (state.countdown > 0) {
            const numLit = Math.min(5, Math.floor((3.0 - state.countdown) / 0.5));
            if (numLit > prevNumLit && numLit > 0) {
                if (!muted) playCountdownTick();
            }
            prevNumLit = numLit;
            prevCountdownActive = true;
        } else if (prevCountdownActive) {
            if (!muted) playCountdownGo();
            prevNumLit = -1;
            prevCountdownActive = false;
        }

        // Win / lose
        if (!gameOver && (state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE)) {
            const won = state.score[myIdx] >= WINNING_SCORE;
            if (!muted) won ? playWin() : playLose();
        }
    }

    // ── Canvas ────────────────────────────────────────────────────────────────
    let canvas: HTMLCanvasElement;
    let rafId = 0;
    let resizeFn: () => void;

    // ── Draw ──────────────────────────────────────────────────────────────────
    function draw(ts: number) {
        rafId = requestAnimationFrame(draw);
        if (!canvas) return;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;

        // Flush buffered pointer position — one IPC call per frame instead of per event
        if (pendingPtr) {
            const [x, y] = pendingPtr;
            pendingPtr = null;
            invoke("set_pointer", { x, y }).catch(() => {});
        }

        const dt = lastTime ? Math.min((ts - lastTime) / 1000, 0.05) : 0;
        lastTime = ts;

        // Update trail
        const dx = rs.puck[0] - lastPuckX,
            dy = rs.puck[1] - lastPuckY;
        if (dx * dx + dy * dy > 4) {
            trail.unshift({ x: rs.puck[0], y: rs.puck[1], age: 0 });
            lastPuckX = rs.puck[0];
            lastPuckY = rs.puck[1];
        }
        for (const t of trail) t.age += dt;
        trail = trail.filter((t) => t.age < 0.1).slice(0, 6);

        const cw = canvas.width,
            ch = canvas.height;
        const scale = Math.min(cw / TW, ch / TH) * 0.92;
        const ox = (cw - TW * scale) / 2,
            oy = (ch - TH * scale) / 2;

        ctx.clearRect(0, 0, cw, ch);
        ctx.fillStyle = "#060b14";
        ctx.fillRect(0, 0, cw, ch);

        ctx.save();
        ctx.translate(ox, oy);
        ctx.scale(scale, scale);
        if (!isHost && !isSinglePlayer) {
            ctx.translate(TW, TH);
            ctx.scale(-1, -1);
        }

        drawTable(ctx);
        drawTrail(ctx);
        drawPuck(ctx);
        drawPaddle(ctx, rs.host_paddle, "#3b82f6", "#93c5fd");
        drawPaddle(ctx, rs.client_paddle, "#10b981", "#6ee7b7");

        if (rs.goal_flash > 0) {
            ctx.fillStyle = `rgba(255,255,255,${rs.goal_flash * 0.25})`;
            ctx.fillRect(0, 0, TW, TH);
        }
        ctx.restore();
        drawHUD(ctx, cw, ch);
    }

    function drawTable(ctx: CanvasRenderingContext2D) {
        const bg = ctx.createLinearGradient(0, 0, 0, TH);
        bg.addColorStop(0, "#091220");
        bg.addColorStop(0.5, "#0d1b33");
        bg.addColorStop(1, "#091220");
        ctx.fillStyle = bg;
        ctx.fillRect(0, 0, TW, TH);

        ctx.strokeStyle = "rgba(59,130,246,0.09)";
        ctx.lineWidth = 0.8;
        for (let x = 0; x <= TW; x += 36) {
            ctx.beginPath();
            ctx.moveTo(x, 0);
            ctx.lineTo(x, TH);
            ctx.stroke();
        }
        for (let y = 0; y <= TH; y += 36) {
            ctx.beginPath();
            ctx.moveTo(0, y);
            ctx.lineTo(TW, y);
            ctx.stroke();
        }

        ctx.strokeStyle = "rgba(244,63,94,0.22)";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(TW / 2, TH / 2, 55, 0, Math.PI * 2);
        ctx.stroke();

        const wf = rs.wall_flash;
        ctx.strokeStyle = `rgba(244,63,94,${0.35 + wf * 0.65})`;
        ctx.lineWidth = 2.5;
        ctx.shadowColor = `rgba(244,63,94,${wf * 0.9})`;
        ctx.shadowBlur = wf * 14;
        ctx.setLineDash([14, 8]);
        ctx.beginPath();
        ctx.moveTo(0, TH / 2);
        ctx.lineTo(TW, TH / 2);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.shadowBlur = 0;

        const RC = CR - 2;
        ctx.shadowColor = `rgba(96,165,250,${wf})`;
        ctx.shadowBlur = wf * 18;
        ctx.strokeStyle = `rgba(148,163,184,${0.55 + wf * 0.45})`;
        ctx.lineWidth = 3.5;

        ctx.beginPath();
        ctx.moveTo(CR, 2);
        ctx.lineTo(GX, 2);
        ctx.moveTo(GX + GOAL_W, 2);
        ctx.lineTo(TW - CR, 2);
        ctx.arc(TW - CR, CR, RC, -Math.PI / 2, 0);
        ctx.lineTo(TW - 2, TH - CR);
        ctx.arc(TW - CR, TH - CR, RC, 0, Math.PI / 2);
        ctx.lineTo(GX + GOAL_W, TH - 2);
        ctx.moveTo(GX, TH - 2);
        ctx.lineTo(CR, TH - 2);
        ctx.arc(CR, TH - CR, RC, Math.PI / 2, Math.PI);
        ctx.lineTo(2, CR);
        ctx.arc(CR, CR, RC, Math.PI, Math.PI * 1.5);
        ctx.stroke();
        ctx.shadowBlur = 0;

        const wg = wf * 0.4;
        const line = (x1: number, y1: number, x2: number, y2: number) => {
            ctx.beginPath();
            ctx.moveTo(x1, y1);
            ctx.lineTo(x2, y2);
            ctx.stroke();
        };
        ctx.fillStyle = `rgba(16,185,129,${0.13 + wg})`;
        ctx.fillRect(GX, 0, GOAL_W, 14);
        ctx.strokeStyle = `rgba(16,185,129,${0.7 + wg})`;
        ctx.lineWidth = 2.5;
        line(GX, 0, GX, 14);
        line(GX + GOAL_W, 0, GX + GOAL_W, 14);

        ctx.fillStyle = `rgba(59,130,246,${0.13 + wg})`;
        ctx.fillRect(GX, TH - 14, GOAL_W, 14);
        ctx.strokeStyle = `rgba(59,130,246,${0.7 + wg})`;
        ctx.lineWidth = 2.5;
        line(GX, TH, GX, TH - 14);
        line(GX + GOAL_W, TH, GX + GOAL_W, TH - 14);
    }

    function drawTrail(ctx: CanvasRenderingContext2D) {
        for (let i = trail.length - 1; i >= 0; i--) {
            const t = trail[i],
                a = (1 - t.age / 0.1) * 0.55,
                r = PR * (1 - t.age / 0.1) * 0.75;
            ctx.beginPath();
            ctx.arc(t.x, t.y, r, 0, Math.PI * 2);
            ctx.fillStyle = `rgba(254,220,50,${a})`;
            ctx.fill();
        }
    }

    function drawPuck(ctx: CanvasRenderingContext2D) {
        const [px, py] = rs.puck;
        const sg = Math.min(rs.puck_speed / MAX_SPEED, 1);
        ctx.shadowColor = `rgba(254,220,50,${0.6 + sg * 0.4})`;
        ctx.shadowBlur = 12 + sg * 28;
        const g = ctx.createRadialGradient(px - 6, py - 6, 2, px, py, PR);
        g.addColorStop(0, "#fff8c2");
        g.addColorStop(0.5, "#fde047");
        g.addColorStop(1, "#b45309");
        ctx.beginPath();
        ctx.arc(px, py, PR, 0, Math.PI * 2);
        ctx.fillStyle = g;
        ctx.fill();
        ctx.shadowBlur = 0;
    }

    function drawPaddle(
        ctx: CanvasRenderingContext2D,
        pos: [number, number],
        col: string,
        light: string,
    ) {
        const [x, y] = pos;
        ctx.shadowColor = col;
        ctx.shadowBlur = 18;
        const g = ctx.createRadialGradient(x - 10, y - 10, 4, x, y, PAR);
        g.addColorStop(0, light);
        g.addColorStop(1, col);
        ctx.beginPath();
        ctx.arc(x, y, PAR, 0, Math.PI * 2);
        ctx.fillStyle = g;
        ctx.fill();
        ctx.strokeStyle = "rgba(255,255,255,0.28)";
        ctx.lineWidth = 2;
        ctx.stroke();
        ctx.shadowBlur = 0;
    }

    function drawHUD(ctx: CanvasRenderingContext2D, cw: number, ch: number) {
        const myIdx = isHost ? 0 : 1,
            opIdx = isHost ? 1 : 0;
        const myCol = isHost ? "#60a5fa" : "#34d399";
        const opCol = isHost ? "#34d399" : "#60a5fa";

        // Score with glow and background pill
        const opSz = 72 + rs.score_flash[opIdx] * 28;
        ctx.font = `900 ${opSz}px system-ui`;
        ctx.textAlign = "left";
        ctx.textBaseline = "bottom";
        
        // Background pill for opponent score
        if (rs.score_flash[opIdx] > 0) {
            ctx.shadowColor = opCol;
            ctx.shadowBlur = 28;
        }
        const opText = String(rs.score[opIdx]);
        const opW = ctx.measureText(opText).width;
        ctx.fillStyle = "rgba(0,0,0,0.4)";
        ctx.beginPath();
        ctx.roundRect(14, ch/2 - 8 - opSz - 8, opW + 24, opSz + 16, 12);
        ctx.fill();
        
        ctx.fillStyle = `rgba(${isHost ? "52,211,153" : "96,165,250"},${0.5 + rs.score_flash[opIdx] * 0.5})`;
        ctx.fillText(opText, 20, ch / 2 - 8);
        ctx.shadowBlur = 0;

        const mySz = 72 + rs.score_flash[myIdx] * 28;
        ctx.font = `900 ${mySz}px system-ui`;
        ctx.textBaseline = "top";
        
        // Background pill for my score
        if (rs.score_flash[myIdx] > 0) {
            ctx.shadowColor = myCol;
            ctx.shadowBlur = 28;
        }
        const myText = String(rs.score[myIdx]);
        const myW = ctx.measureText(myText).width;
        ctx.fillStyle = "rgba(0,0,0,0.4)";
        ctx.beginPath();
        ctx.roundRect(14, ch/2 + 8 - 8, myW + 24, mySz + 16, 12);
        ctx.fill();
        
        ctx.fillStyle = `rgba(${isHost ? "96,165,250" : "52,211,153"},${0.85 + rs.score_flash[myIdx] * 0.15})`;
        ctx.fillText(myText, 20, ch / 2 + 8);
        ctx.shadowBlur = 0;

        if (rs.countdown > 0) {
            // F1-style lights: one light turns on every 0.5s, all go dark at 0 → GO
            const numLit = Math.min(5, Math.floor((3.0 - rs.countdown) / 0.5));
            const LIGHTS = 5, R = Math.round(cw * 0.055), GAP = Math.round(R * 0.55);
            const totalW = LIGHTS * 2 * R + (LIGHTS - 1) * GAP;
            const lx0 = cw / 2 - totalW / 2 + R;
            const ly  = ch * 0.42;

            // Dark panel
            const pw = totalW + R * 1.6, ph = R * 2.8;
            ctx.fillStyle = "rgba(0,0,0,0.82)";
            ctx.beginPath();
            ctx.roundRect(cw / 2 - pw / 2, ly - ph / 2, pw, ph, R * 0.35);
            ctx.fill();

            // Gantry bar
            ctx.fillStyle = "#1a1a1a";
            ctx.fillRect(cw / 2 - pw / 2, ly - 2, pw, 4);

            for (let i = 0; i < LIGHTS; i++) {
                const lxi = lx0 + i * (2 * R + GAP);
                const lit = i < numLit;

                if (lit) {
                    // Red glow halo
                    const g = ctx.createRadialGradient(lxi, ly, 0, lxi, ly, R * 2.2);
                    g.addColorStop(0, "rgba(255,30,0,0.55)");
                    g.addColorStop(1, "rgba(255,0,0,0)");
                    ctx.fillStyle = g;
                    ctx.beginPath();
                    ctx.arc(lxi, ly, R * 2.2, 0, Math.PI * 2);
                    ctx.fill();
                    ctx.shadowColor = "#ff2200";
                    ctx.shadowBlur = R;
                    ctx.fillStyle = "#ff2200";
                } else {
                    ctx.shadowBlur = 0;
                    ctx.fillStyle = "#220800";
                }

                ctx.beginPath();
                ctx.arc(lxi, ly, R, 0, Math.PI * 2);
                ctx.fill();
                ctx.shadowBlur = 0;

                // Housing ring
                ctx.strokeStyle = lit ? "#ff6644" : "#3a1a14";
                ctx.lineWidth = Math.max(1.5, R * 0.1);
                ctx.beginPath();
                ctx.arc(lxi, ly, R, 0, Math.PI * 2);
                ctx.stroke();
            }
        }
    }

    async function rematch() {
        gameOver = false;
        prevScore = [0, 0]; prevNumLit = -1; prevCountdownActive = true; puckNearMyGoal = false; nearMissCooldown = 0;
        rs = { puck:[TW/2,TH/2], puck_speed:0, host_paddle:[TW/2,TH-120], client_paddle:[TW/2,120],
               score:[0,0], wall_flash:0, goal_flash:0, score_flash:[0,0], hit:0, wall_hit:0, goal_scored:0, countdown:3 };
        const ch = new Channel<RS>();
        ch.onmessage = (state) => {
            handleAudio(state);
            rs = state;
            if (!gameOver && (state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE)) {
                gameOver = true;
                const myIdx = isHost ? 0 : 1;
                iWon = state.score[myIdx] >= WINNING_SCORE;
                invoke("stop_game").catch(() => {});
                showAd();
            }
        };
        await invoke("start_game", { isHost, isSinglePlayer, channel: ch, useUdp });
    }

    // ── Input → Rust ──────────────────────────────────────────────────────────
    function tableCoords(cx: number, cy: number): [number, number] {
        const dpr = window.devicePixelRatio || 1;
        const cw = canvas.width,
            ch = canvas.height;
        const scale = Math.min(cw / TW, ch / TH) * 0.92;
        const ox = (cw - TW * scale) / 2,
            oy = (ch - TH * scale) / 2;
        let x = (cx * dpr - ox) / scale,
            y = (cy * dpr - oy) / scale;
        if (!isHost && !isSinglePlayer) {
            x = TW - x;
            y = TH - y;
        }
        return [x, y];
    }

    function onPointerMove(e: PointerEvent) {
        e.preventDefault();
        const [x, y] = tableCoords(e.clientX, e.clientY);
        // Buffer position — flushed to Rust once per rAF frame in draw()
        pendingPtr = [x, y];
    }

    // ── Mount ─────────────────────────────────────────────────────────────────
    onMount(async () => {
        preloadAd(); // start loading while the game initialises
        const dpr = window.devicePixelRatio || 1;
        resizeFn = () => {
            canvas.width = window.innerWidth * dpr;
            canvas.height = window.innerHeight * dpr;
            canvas.style.width = window.innerWidth + "px";
            canvas.style.height = window.innerHeight + "px";
        };
        resizeFn();
        window.addEventListener("resize", resizeFn);
        canvas.addEventListener("pointermove", onPointerMove, { passive: false });
        canvas.addEventListener("pointerdown", onPointerMove, { passive: false });

        const onVisibility = () => {
            if (document.hidden) invoke("pause_game").catch(() => {});
            else invoke("resume_game").catch(() => {});
        };
        document.addEventListener("visibilitychange", onVisibility);

        // Channel: Rust pushes RenderState here every ~16ms
        const ch = new Channel<RS>();
        ch.onmessage = (state) => {
            handleAudio(state);
            rs = state;
            if (!gameOver && (state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE)) {
                gameOver = true;
                const myIdx = isHost ? 0 : 1;
                iWon = state.score[myIdx] >= WINNING_SCORE;
                invoke("stop_game").catch(() => {});
                showAd();
            }
        };

        // Ensure any lingering game loop from a previous session is stopped
        // before starting a new one (running flag could be stuck true).
        await invoke("stop_game").catch(() => {});

        // Start Rust game engine
        try {
            await invoke("start_game", { isHost, isSinglePlayer, channel: ch, useUdp });
        } catch (e) {
            console.error("start_game failed:", e);
            onBack?.();
            return;
        }

        rafId = requestAnimationFrame(draw);
    });

    onDestroy(async () => {
        cancelAnimationFrame(rafId);
        if (resizeFn) window.removeEventListener("resize", resizeFn);
        document.removeEventListener("visibilitychange", () => {});
        await invoke("stop_game").catch(() => {});
    });
</script>

<canvas
    bind:this={canvas}
    class="touch-none block bg-gradient-to-b from-[#0a0e1a] via-[#060b14] to-[#0a0e1a]"
    style="width: 100vw; height: 100vh; position: fixed; top: 0; left: 0;"
></canvas>

{#if gameOver}
<div class="fixed inset-0 flex flex-col items-center justify-center z-10 bg-black/80 backdrop-blur-md">
    <div class="flex flex-col items-center gap-6 p-12 rounded-[2.5rem] bg-gradient-to-br from-neutral-900/95 to-neutral-800/90 border border-neutral-600/50 shadow-[0_0_60px_rgba(0,0,0,0.6)] animate-in fade-in zoom-in duration-300">
        <div class="text-7xl {iWon ? 'animate-bounce' : 'animate-pulse'}">{iWon ? '🏆' : '😔'}</div>
        <h2 class="text-5xl font-black {iWon ? 'text-yellow-400 drop-shadow-[0_0_24px_rgba(250,204,21,0.6)]' : 'text-neutral-400'}">{iWon ? 'VICTORY!' : 'DEFEAT'}</h2>
        <div class="flex items-center gap-4 text-4xl font-black text-white tracking-widest bg-neutral-800/50 px-8 py-4 rounded-2xl border border-neutral-600/30">
            <span class="{rs.score[0] > rs.score[1] ? 'text-yellow-400' : 'text-neutral-400'}">{rs.score[0]}</span>
            <span class="text-neutral-600">–</span>
            <span class="{rs.score[1] > rs.score[0] ? 'text-yellow-400' : 'text-neutral-400'}">{rs.score[1]}</span>
        </div>
        <div class="flex gap-3 mt-4 w-full">
            {#if isSinglePlayer}
            <button class="flex-1 py-4 bg-gradient-to-r from-emerald-600 to-emerald-500 text-white rounded-2xl text-lg font-bold hover:from-emerald-500 hover:to-emerald-400 active:scale-95 uppercase tracking-widest shadow-[0_0_24px_rgba(16,185,129,0.4)] border border-emerald-400/30 transition-all" onclick={rematch}>🔄 Play Again</button>
            {/if}
            <button class="flex-1 py-4 bg-gradient-to-r from-neutral-700 to-neutral-600 text-white rounded-2xl text-lg font-bold hover:from-neutral-600 hover:to-neutral-500 active:scale-95 uppercase tracking-widest shadow-lg border border-neutral-500/30 transition-all" onclick={() => onBack?.()}>🏠 Menu</button>
        </div>
    </div>
</div>
{:else}
<button
    class="fixed top-4 left-4 z-10 w-11 h-11 bg-black/50 text-white/70 rounded-full text-xl flex items-center justify-center active:scale-90 hover:text-white hover:bg-black/70 backdrop-blur-sm border border-white/10 shadow-lg transition-all"
    onclick={() => { invoke('stop_game').catch(() => {}); onBack?.(); }}
>✕</button>
<button
    class="fixed top-4 right-4 z-10 w-11 h-11 bg-black/50 text-white/70 rounded-full text-xl flex items-center justify-center active:scale-90 hover:text-white hover:bg-black/70 backdrop-blur-sm border border-white/10 shadow-lg transition-all"
    onclick={() => muted = !muted}
>{muted ? '🔇' : '🔊'}</button>
{/if}
