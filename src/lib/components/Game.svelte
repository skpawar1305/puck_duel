<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { Channel } from "@tauri-apps/api/core";
    import { playHit, playWall, playGoal, initAudio } from "$lib/audio";

    let {
        isHost,
        isSinglePlayer = false,
        onBack,
    } = $props<{
        isHost: boolean;
        isSinglePlayer?: boolean;
        onBack?: () => void;
    }>();

    const WINNING_SCORE = 6;

    // ── Constants (render only) ───────────────────────────────────────────────
    const TW = 360,
        TH = 640;
    const PR = 20,
        PAR = 27;
    const GOAL_W = 110;
    const CR = 42;
    const MAX_SPEED = 900;
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

    // Trail maintained locally (JS side) from puck position updates
    let trail: { x: number; y: number; age: number }[] = [];
    let lastPuckX = TW / 2,
        lastPuckY = TH / 2;
    let lastTime = 0;

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
        ctx.textAlign = "center";

        const opSz = 28 + rs.score_flash[opIdx] * 20;
        ctx.font = `900 ${opSz}px system-ui`;
        ctx.textBaseline = "top";
        if (rs.score_flash[opIdx] > 0) {
            ctx.shadowColor = opCol;
            ctx.shadowBlur = 22;
        }
        ctx.fillStyle = `rgba(${isHost ? "52,211,153" : "96,165,250"},${0.45 + rs.score_flash[opIdx] * 0.55})`;
        ctx.fillText(String(rs.score[opIdx]), cw / 2, 14);
        ctx.shadowBlur = 0;

        const mySz = 28 + rs.score_flash[myIdx] * 20;
        ctx.font = `900 ${mySz}px system-ui`;
        ctx.textBaseline = "bottom";
        if (rs.score_flash[myIdx] > 0) {
            ctx.shadowColor = myCol;
            ctx.shadowBlur = 22;
        }
        ctx.fillStyle = `rgba(${isHost ? "96,165,250" : "52,211,153"},${0.8 + rs.score_flash[myIdx] * 0.2})`;
        ctx.fillText(String(rs.score[myIdx]), cw / 2, ch - 14);
        ctx.shadowBlur = 0;

        if (rs.countdown > 0) {
            const n = Math.ceil(rs.countdown);
            const pulse = rs.countdown % 1;
            ctx.font = `900 ${100 - pulse * 30}px system-ui`;
            ctx.textBaseline = "middle";
            ctx.textAlign = "center";
            ctx.fillStyle = `rgba(255,255,255,${0.4 + pulse * 0.6})`;
            ctx.shadowColor = "rgba(255,255,255,0.7)";
            ctx.shadowBlur = 30;
            ctx.fillText(String(n), cw / 2, ch / 2);
            ctx.shadowBlur = 0;
        }
    }

    async function rematch() {
        gameOver = false;
        rs = { puck:[TW/2,TH/2], puck_speed:0, host_paddle:[TW/2,TH-120], client_paddle:[TW/2,120],
               score:[0,0], wall_flash:0, goal_flash:0, score_flash:[0,0], hit:0, wall_hit:0, goal_scored:0, countdown:3 };
        const ch = new Channel<RS>();
        ch.onmessage = (state) => {
            if (state.hit) { if (!muted) playHit(); navigator.vibrate?.(12); }
            if (state.wall_hit) { if (!muted) playWall(); navigator.vibrate?.(6); }
            if (state.goal_scored) { if (!muted) playGoal(); navigator.vibrate?.([40, 30, 40]); }
            rs = state;
            if (!gameOver && (state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE)) {
                gameOver = true;
                const myIdx = isHost ? 0 : 1;
                iWon = state.score[myIdx] >= WINNING_SCORE;
                invoke("stop_game").catch(() => {});
            }
        };
        await invoke("start_game", { isHost, isSinglePlayer, channel: ch });
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
        // Fire-and-forget — no await, set_pointer is a trivial Mutex write in Rust
        invoke("set_pointer", { x, y }).catch(() => {});
    }

    // ── Mount ─────────────────────────────────────────────────────────────────
    onMount(async () => {
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
            if (state.hit) { if (!muted) playHit(); navigator.vibrate?.(12); }
            if (state.wall_hit) { if (!muted) playWall(); navigator.vibrate?.(6); }
            if (state.goal_scored) { if (!muted) playGoal(); navigator.vibrate?.([40, 30, 40]); }
            rs = state;
            if (!gameOver && (state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE)) {
                gameOver = true;
                const myIdx = isHost ? 0 : 1;
                iWon = state.score[myIdx] >= WINNING_SCORE;
                invoke("stop_game").catch(() => {});
            }
        };

        // Start Rust game engine
        await invoke("start_game", {
            isHost,
            isSinglePlayer,
            channel: ch,
        });

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
    class="touch-none block bg-[#060b14]"
    style="width: 100vw; height: 100vh; position: fixed; top: 0; left: 0;"
></canvas>

{#if gameOver}
<div class="fixed inset-0 flex flex-col items-center justify-center z-10 bg-black/70 backdrop-blur-sm">
    <div class="flex flex-col items-center gap-6 p-10 rounded-3xl bg-neutral-900/90 border border-neutral-700 shadow-2xl">
        <div class="text-6xl">{iWon ? '🏆' : '😔'}</div>
        <h2 class="text-4xl font-black {iWon ? 'text-yellow-400' : 'text-neutral-400'}">{iWon ? 'YOU WIN!' : 'YOU LOSE'}</h2>
        <p class="text-3xl font-bold text-white tracking-widest">{rs.score[0]} – {rs.score[1]}</p>
        <div class="flex gap-3 mt-2 w-full">
            {#if isSinglePlayer}
            <button class="flex-1 py-4 bg-emerald-600 text-white rounded-2xl text-lg font-bold hover:bg-emerald-500 uppercase tracking-widest" onclick={rematch}>▶ Play Again</button>
            {/if}
            <button class="flex-1 py-4 bg-neutral-700 text-white rounded-2xl text-lg font-bold hover:bg-neutral-600 uppercase tracking-widest" onclick={() => onBack?.()}>Menu</button>
        </div>
    </div>
</div>
{:else}
<button
    class="fixed top-3 left-3 z-10 w-10 h-10 bg-black/40 text-white/60 rounded-full text-lg flex items-center justify-center active:scale-90 hover:text-white/90 hover:bg-black/60"
    onclick={() => { invoke('stop_game').catch(() => {}); onBack?.(); }}
>✕</button>
<button
    class="fixed top-3 right-3 z-10 w-10 h-10 bg-black/40 text-white/60 rounded-full text-lg flex items-center justify-center active:scale-90 hover:text-white/90 hover:bg-black/60"
    onclick={() => muted = !muted}
>{muted ? '🔇' : '🔊'}</button>
{/if}
