<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { Channel } from "@tauri-apps/api/core";
    import { Application, Container, Sprite, Graphics, Text, TextStyle, Texture } from "pixi.js";
    import {
        playHit,
        playWall,
        playMyGoal,
        playOpponentGoal,
        playNearMiss,
        playCountdownTick,
        playCountdownGo,
        playWin,
        playLose,
        initAudio,
    } from "$lib/audio";
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
        preloadAd();
        if (!ad) return;
        try {
            await ad.show();
        } catch {
            // Ad failure is non-fatal.
        }
    }

    const TW = 360,
        TH = 640;
    const PR = 20,
        PAR = 27;
    const GOAL_W = 110;
    const CR = 42;
    const MAX_SPEED = 990;
    const GX = (TW - GOAL_W) / 2;

    const COL = {
        bgTop: "#020914",
        bgMid: "#0a1a2e",
        bgBottom: "#040a13",
        lane: "rgba(159,215,255,0.14)",
        stripe: "rgba(145,192,232,0.12)",
        borderCore: 0xd7e4f2,
        borderAccent: 0x9fd3ff,
        blue: 0x69b6ff,
        green: 0x38d7a5,
        puck: 0xe9ecef,
        champagne: 0xe8edf5,
    } as const;

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

    let rs = $state<RS>({
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
    });

    let gameOver = $state(false);
    let iWon = $state(false);
    let muted = $state(false);

    let pendingPtr: [number, number] | null = null;
    let localPaddlePos: [number, number] | null = null;

    let trail: { x: number; y: number; age: number }[] = [];
    let lastPuckX = TW / 2,
        lastPuckY = TH / 2;
    let lastTime = 0;

    let app: Application | null = null;
    let containerEl: HTMLDivElement;
    let dpr = 1;
    let pxW = 0,
        pxH = 0;
    let scale = 1,
        ox = 0,
        oy = 0;

    let tableContainer: Container;
    let borderGlowGfx: Graphics;
    let midlineGlowGfx: Graphics;
    let wallFlashGfx: Graphics;
    let borderRunnerGfx: Graphics;
    let borderGfx: Graphics;
    let goalsGfx: Graphics;
    let midlineGfx: Graphics;
    let trailGfx: Graphics;
    let puckFxGfx: Graphics;
    let paddleFxGfx: Graphics;
    let puckDecalGfx: Graphics;
    let paddleDecalGfx: Graphics;
    let actorFallbackGfx: Graphics;
    let puckBodySpr: Sprite;
    let puckIconSpr: Sprite;
    let paddleBlueSpr: Sprite;
    let paddleGreenSpr: Sprite;
    let paddleBlueIconSpr: Sprite;
    let paddleGreenIconSpr: Sprite;
    let goalFlashGfx: Graphics;

    let hudContainer: Container;
    let scoreGlowGfx: Graphics;
    let opScoreBg: Graphics;
    let opScoreText: Text;
    let myScoreBg: Graphics;
    let myScoreText: Text;
    let countdownContainer: Container;
    let countdownPanel: Graphics;
    let countdownLightSprites: Sprite[] = [];

    let litLightTex: Texture;
    let unlitLightTex: Texture;
    let litLightGlowTex: Texture;

    const PAD_SPRITE_SIZE = (PAR + 22) * 2;
    const PUCK_SPRITE_SIZE = (PR + 4) * 2;

    let onVisibility: (() => void) | null = null;
    let mountedCanvas: HTMLCanvasElement | null = null;
    let audioUnlocked = false;
    let tableFxEnabled = true;
    let actorFallbackEnabled = false;

    function unlockAudioOnce() {
        if (audioUnlocked) return;
        audioUnlocked = true;
        try {
            initAudio();
        } catch {
            // Non-fatal: audio will retry on next allowed gesture.
            audioUnlocked = false;
        }
    }

    function hexToRgba(hex: string, a: number): string {
        const r = parseInt(hex.slice(1, 3), 16);
        const g = parseInt(hex.slice(3, 5), 16);
        const b = parseInt(hex.slice(5, 7), 16);
        return `rgba(${r},${g},${b},${a})`;
    }

    function makeCanvas(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
        const c = document.createElement("canvas");
        c.width = w;
        c.height = h;
        return [c, c.getContext("2d")!];
    }

    function buildPaddleTexture(col: string, light: string): Texture {
        const size = PAD_SPRITE_SIZE;
        const [c, s] = makeCanvas(size, size);
        const cx = size / 2,
            cy = size / 2;

        // Soft floor shadow helps paddles feel anchored to the rink.
        s.fillStyle = "rgba(2,8,20,0.55)";
        s.beginPath();
        s.ellipse(cx, cy + PAR * 0.3, PAR * 0.95, PAR * 0.52, 0, 0, Math.PI * 2);
        s.fill();

        const glowGrad = s.createRadialGradient(cx, cy, PAR * 0.4, cx, cy, size / 2);
        glowGrad.addColorStop(0, hexToRgba(col, 0.75));
        glowGrad.addColorStop(0.62, hexToRgba(col, 0.34));
        glowGrad.addColorStop(1, hexToRgba(col, 0));
        s.fillStyle = glowGrad;
        s.beginPath();
        s.arc(cx, cy, size / 2, 0, Math.PI * 2);
        s.fill();

        const g = s.createRadialGradient(cx - 8, cy - 10, 3, cx, cy, PAR);
        g.addColorStop(0, "#ffffff");
        g.addColorStop(0.22, light);
        g.addColorStop(0.62, col);
        g.addColorStop(1, "#081126");
        s.beginPath();
        s.arc(cx, cy, PAR, 0, Math.PI * 2);
        s.fillStyle = g;
        s.fill();

        s.strokeStyle = "rgba(255,255,255,0.42)";
        s.lineWidth = 2.4;
        s.stroke();

        s.strokeStyle = "rgba(255,255,255,0.24)";
        s.lineWidth = 0.9;
        s.beginPath();
        s.arc(cx, cy, PAR - 5.4, 0, Math.PI * 2);
        s.stroke();

        s.strokeStyle = hexToRgba(col, 0.65);
        s.lineWidth = 5;
        s.beginPath();
        s.arc(cx, cy, PAR - 1.2, 0, Math.PI * 2);
        s.stroke();

        const inner = s.createRadialGradient(cx - 4, cy - 4, 0, cx, cy, PAR * 0.55);
        inner.addColorStop(0, "rgba(255,255,255,0.95)");
        inner.addColorStop(1, hexToRgba(col, 0));
        s.fillStyle = inner;
        s.beginPath();
        s.arc(cx, cy, PAR * 0.55, 0, Math.PI * 2);
        s.fill();

        s.strokeStyle = "rgba(255,255,255,0.25)";
        s.lineWidth = 1.2;
        s.beginPath();
        s.arc(cx, cy, PAR * 0.52, 0, Math.PI * 2);
        s.stroke();

        // Bold segmented ring so paddles read as multi-tone, not flat circles.
        s.lineCap = "round";
        s.lineWidth = 3.1;
        s.strokeStyle = "rgba(255,255,255,0.72)";
        s.beginPath();
        s.arc(cx, cy, PAR * 0.7, -Math.PI * 0.18, Math.PI * 0.32);
        s.stroke();
        s.beginPath();
        s.arc(cx, cy, PAR * 0.7, Math.PI * 0.82, Math.PI * 1.28);
        s.stroke();

        s.strokeStyle = "rgba(6,12,24,0.76)";
        s.lineWidth = 2.6;
        s.beginPath();
        s.arc(cx, cy, PAR * 0.7, Math.PI * 0.36, Math.PI * 0.74);
        s.stroke();
        s.beginPath();
        s.arc(cx, cy, PAR * 0.7, Math.PI * 1.36, Math.PI * 1.74);
        s.stroke();

        // Center cap gives immediate depth cue.
        s.fillStyle = "rgba(255,255,255,0.88)";
        s.beginPath();
        s.arc(cx, cy, PAR * 0.18, 0, Math.PI * 2);
        s.fill();
        s.fillStyle = "rgba(0,0,0,0.45)";
        s.beginPath();
        s.arc(cx + 1.2, cy + 1.2, PAR * 0.1, 0, Math.PI * 2);
        s.fill();

        return Texture.from(c);
    }

    function buildPuckBodyTexture(): Texture {
        const size = PUCK_SPRITE_SIZE;
        const [c, s] = makeCanvas(size, size);
        const cx = size / 2,
            cy = size / 2;

        // Tight, high-contrast puck body without oversized decorative radius.
        const g = s.createRadialGradient(cx - 3, cy - 4, 2, cx, cy, PR);
        g.addColorStop(0, "#f8fbff");
        g.addColorStop(0.3, "#d7dfe7");
        g.addColorStop(0.72, "#7b8591");
        g.addColorStop(1, "#151a21");
        s.beginPath();
        s.arc(cx, cy, PR, 0, Math.PI * 2);
        s.fillStyle = g;
        s.fill();

        const sheen = s.createLinearGradient(cx - PR, cy - PR, cx + PR, cy + PR);
        sheen.addColorStop(0, "rgba(255,255,255,0.24)");
        sheen.addColorStop(0.5, "rgba(255,255,255,0)");
        sheen.addColorStop(1, "rgba(255,255,255,0.16)");
        s.fillStyle = sheen;
        s.beginPath();
        s.arc(cx, cy, PR * 0.96, 0, Math.PI * 2);
        s.fill();

        const ring = s.createRadialGradient(cx, cy, PR * 0.26, cx, cy, PR);
        ring.addColorStop(0, "rgba(255,255,255,0)");
        ring.addColorStop(0.82, "rgba(232,236,242,0)");
        ring.addColorStop(1, "rgba(232,236,242,0.58)");
        s.fillStyle = ring;
        s.beginPath();
        s.arc(cx, cy, PR, 0, Math.PI * 2);
        s.fill();

        s.fillStyle = "rgba(255,255,255,0.55)";
        s.beginPath();
        s.arc(cx - 6, cy - 6, PR * 0.2, 0, Math.PI * 2);
        s.fill();

        s.strokeStyle = "rgba(229,237,247,0.85)";
        s.lineWidth = 1.3;
        s.beginPath();
        s.arc(cx, cy, PR - 1.1, 0, Math.PI * 2);
        s.stroke();

        // Dark seam ring to break the "flat coin" silhouette.
        s.strokeStyle = "rgba(15,19,24,0.92)";
        s.lineWidth = 2.1;
        s.beginPath();
        s.arc(cx, cy, PR * 0.68, 0, Math.PI * 2);
        s.stroke();

        // Bright core ring for immediate readability at small sizes.
        s.strokeStyle = "rgba(245,250,255,0.92)";
        s.lineWidth = 1.4;
        s.beginPath();
        s.arc(cx, cy, PR * 0.38, 0, Math.PI * 2);
        s.stroke();

        // Segmented slashes add texture and stop the puck from reading as mono.
        s.save();
        s.translate(cx, cy);
        s.lineCap = "round";
        for (let i = 0; i < 6; i++) {
            const a = (Math.PI * 2 * i) / 6 + Math.PI * 0.08;
            const x0 = Math.cos(a) * (PR * 0.42);
            const y0 = Math.sin(a) * (PR * 0.42);
            const x1 = Math.cos(a) * (PR * 0.62);
            const y1 = Math.sin(a) * (PR * 0.62);

            s.strokeStyle = "rgba(196,211,228,0.86)";
            s.lineWidth = 1.7;
            s.beginPath();
            s.moveTo(x0, y0);
            s.lineTo(x1, y1);
            s.stroke();

            s.strokeStyle = "rgba(28,34,42,0.76)";
            s.lineWidth = 0.8;
            s.beginPath();
            s.moveTo(x0 * 0.96, y0 * 0.96);
            s.lineTo(x1 * 0.96, y1 * 0.96);
            s.stroke();
        }
        s.restore();

        // High-contrast split arcs: clearly visible even on small mobile screens.
        s.lineCap = "round";
        s.lineWidth = 3.2;
        s.strokeStyle = "rgba(251,253,255,0.82)";
        s.beginPath();
        s.arc(cx, cy, PR * 0.84, -Math.PI * 0.06, Math.PI * 0.44);
        s.stroke();
        s.beginPath();
        s.arc(cx, cy, PR * 0.84, Math.PI * 0.94, Math.PI * 1.44);
        s.stroke();

        s.strokeStyle = "rgba(18,23,30,0.9)";
        s.beginPath();
        s.arc(cx, cy, PR * 0.84, Math.PI * 0.5, Math.PI * 0.84);
        s.stroke();
        s.beginPath();
        s.arc(cx, cy, PR * 0.84, Math.PI * 1.5, Math.PI * 1.84);
        s.stroke();

        // Center emblem avoids the "single-color blob" look.
        s.fillStyle = "rgba(248,252,255,0.94)";
        s.beginPath();
        s.arc(cx, cy, PR * 0.16, 0, Math.PI * 2);
        s.fill();
        s.strokeStyle = "rgba(24,32,41,0.9)";
        s.lineWidth = 1.2;
        s.beginPath();
        s.moveTo(cx - PR * 0.2, cy);
        s.lineTo(cx + PR * 0.2, cy);
        s.stroke();
        s.beginPath();
        s.moveTo(cx, cy - PR * 0.2);
        s.lineTo(cx, cy + PR * 0.2);
        s.stroke();

        return Texture.from(c);
    }

    function buildPuckIconTexture(): Texture {
        const size = PUCK_SPRITE_SIZE;
        const [c, s] = makeCanvas(size, size);
        const cx = size / 2;

        s.strokeStyle = "rgba(10,10,10,0.95)";
        s.lineWidth = 3;
        s.beginPath();
        s.moveTo(cx - PR * 0.22, cx);
        s.lineTo(cx + PR * 0.22, cx);
        s.stroke();
        s.beginPath();
        s.moveTo(cx, cx - PR * 0.22);
        s.lineTo(cx, cx + PR * 0.22);
        s.stroke();

        s.fillStyle = "rgba(255,255,255,0.96)";
        s.beginPath();
        s.moveTo(cx, cx - PR * 0.33);
        s.lineTo(cx + PR * 0.12, cx);
        s.lineTo(cx, cx + PR * 0.33);
        s.lineTo(cx - PR * 0.12, cx);
        s.closePath();
        s.fill();

        s.strokeStyle = "rgba(0,0,0,0.75)";
        s.lineWidth = 1.2;
        s.stroke();

        return Texture.from(c);
    }

    function buildPaddleIconTexture(primary: string): Texture {
        const size = PAD_SPRITE_SIZE;
        const [c, s] = makeCanvas(size, size);
        const cx = size / 2;
        const r = PAR * 0.5;

        s.strokeStyle = "rgba(0,0,0,0.75)";
        s.lineWidth = 3.2;
        s.beginPath();
        s.arc(cx, cx, r, 0, Math.PI * 2);
        s.stroke();

        s.strokeStyle = "rgba(255,255,255,0.92)";
        s.lineWidth = 1.6;
        s.beginPath();
        s.arc(cx, cx, r, 0, Math.PI * 2);
        s.stroke();

        s.fillStyle = primary;
        s.beginPath();
        s.moveTo(cx, cx - r * 0.6);
        s.lineTo(cx + r * 0.55, cx + r * 0.35);
        s.lineTo(cx - r * 0.55, cx + r * 0.35);
        s.closePath();
        s.fill();

        s.strokeStyle = "rgba(255,255,255,0.96)";
        s.lineWidth = 1.2;
        s.beginPath();
        s.moveTo(cx, cx - r * 0.34);
        s.lineTo(cx + r * 0.25, cx + r * 0.16);
        s.lineTo(cx - r * 0.25, cx + r * 0.16);
        s.closePath();
        s.stroke();

        return Texture.from(c);
    }

    function buildStaticBgTexture(): Texture {
        const [c, s] = makeCanvas(TW, TH);

        const bg = s.createLinearGradient(0, 0, 0, TH);
        bg.addColorStop(0, COL.bgTop);
        bg.addColorStop(0.5, COL.bgMid);
        bg.addColorStop(1, COL.bgBottom);
        s.fillStyle = bg;
        s.fillRect(0, 0, TW, TH);

        const topBloom = s.createRadialGradient(TW / 2, -30, 10, TW / 2, 40, TW * 0.7);
        topBloom.addColorStop(0, "rgba(147,206,255,0.2)");
        topBloom.addColorStop(1, "rgba(147,206,255,0)");
        s.fillStyle = topBloom;
        s.fillRect(0, 0, TW, TH * 0.45);

        // Ice slab gradient inside the board area.
        const ice = s.createLinearGradient(0, 8, 0, TH - 8);
        ice.addColorStop(0, "rgba(214,235,255,0.08)");
        ice.addColorStop(0.5, "rgba(140,185,227,0.12)");
        ice.addColorStop(1, "rgba(214,235,255,0.08)");
        s.fillStyle = ice;
        s.fillRect(6, 6, TW - 12, TH - 12);

        // Longitudinal lane marks.
        s.strokeStyle = COL.lane;
        s.lineWidth = 1.4;
        s.beginPath();
        s.moveTo(30, 20);
        s.lineTo(30, TH - 20);
        s.moveTo(TW - 30, 20);
        s.lineTo(TW - 30, TH - 20);
        s.stroke();

        // Cross-rink stripes.
        s.strokeStyle = COL.stripe;
        s.lineWidth = 1;
        for (let y = 36; y < TH; y += 44) {
            s.beginPath();
            s.moveTo(42, y);
            s.lineTo(TW - 42, y);
            s.stroke();
        }

        // Subtle diagonal sweep for a cleaner premium surface.
        s.strokeStyle = "rgba(225,241,255,0.08)";
        s.lineWidth = 1.2;
        for (let x = -TH; x < TW + TH; x += 46) {
            s.beginPath();
            s.moveTo(x, 0);
            s.lineTo(x + TH * 0.65, TH);
            s.stroke();
        }

        // Faceoff rings and center marks.
        s.strokeStyle = "rgba(242,223,177,0.44)";
        s.lineWidth = 2;
        s.beginPath();
        s.arc(TW / 2, TH / 2, 52, 0, Math.PI * 2);
        s.stroke();

        s.strokeStyle = "rgba(125,221,190,0.34)";
        s.beginPath();
        s.arc(TW / 2, 116, 32, 0, Math.PI * 2);
        s.arc(TW / 2, TH - 116, 32, 0, Math.PI * 2);
        s.stroke();

        s.fillStyle = "rgba(242,223,177,0.62)";
        s.beginPath();
        s.arc(TW / 2, TH / 2, 4, 0, Math.PI * 2);
        s.fill();

        s.fillStyle = "rgba(125,221,190,0.58)";
        s.beginPath();
        s.arc(TW / 2, 116, 3, 0, Math.PI * 2);
        s.arc(TW / 2, TH - 116, 3, 0, Math.PI * 2);
        s.fill();

        // Corner rivet accents frame the board and improve depth.
        s.fillStyle = "rgba(236,247,255,0.25)";
        for (const [x, y] of [[CR, CR], [TW - CR, CR], [CR, TH - CR], [TW - CR, TH - CR]] as [number, number][]) {
            s.beginPath();
            s.arc(x, y, 2.1, 0, Math.PI * 2);
            s.fill();
        }

        // Subtle vignette to keep focus toward center.
        const vignette = s.createRadialGradient(TW / 2, TH / 2, TH * 0.18, TW / 2, TH / 2, TH * 0.72);
        vignette.addColorStop(0, "rgba(0,0,0,0)");
        vignette.addColorStop(1, "rgba(0,0,0,0.34)");
        s.fillStyle = vignette;
        s.fillRect(0, 0, TW, TH);

        return Texture.from(c);
    }

    function buildLightTexture(lit: boolean, R: number): Texture {
        const size = Math.ceil(R * 5);
        const [c, s] = makeCanvas(size, size);
        const cx = size / 2;

        if (lit) {
            const g = s.createRadialGradient(cx, cx, 0, cx, cx, R * 2.2);
            g.addColorStop(0, "rgba(255,30,0,0.55)");
            g.addColorStop(1, "rgba(255,0,0,0)");
            s.fillStyle = g;
            s.beginPath();
            s.arc(cx, cx, R * 2.2, 0, Math.PI * 2);
            s.fill();

            s.shadowColor = "#ff2200";
            s.shadowBlur = R;
            s.fillStyle = "#ff2200";
            s.beginPath();
            s.arc(cx, cx, R, 0, Math.PI * 2);
            s.fill();
            s.shadowBlur = 0;

            s.strokeStyle = "#ff6644";
            s.lineWidth = Math.max(1.5, R * 0.1);
            s.beginPath();
            s.arc(cx, cx, R, 0, Math.PI * 2);
            s.stroke();
        } else {
            s.fillStyle = "#220800";
            s.beginPath();
            s.arc(cx, cx, R, 0, Math.PI * 2);
            s.fill();

            s.strokeStyle = "#3a1a14";
            s.lineWidth = Math.max(1.5, R * 0.1);
            s.beginPath();
            s.arc(cx, cx, R, 0, Math.PI * 2);
            s.stroke();
        }

        return Texture.from(c);
    }

    function buildLightGlowTexture(R: number): Texture {
        const size = Math.ceil(R * 5);
        const [c, s] = makeCanvas(size, size);
        const cx = size / 2;
        const g = s.createRadialGradient(cx, cx, 0, cx, cx, R * 2.2);
        g.addColorStop(0, "rgba(255,40,0,0.58)");
        g.addColorStop(0.6, "rgba(255,40,0,0.18)");
        g.addColorStop(1, "rgba(255,40,0,0)");
        s.fillStyle = g;
        s.beginPath();
        s.arc(cx, cx, R * 2.2, 0, Math.PI * 2);
        s.fill();
        return Texture.from(c);
    }

    let prevScore: [number, number] = [0, 0];
    let prevNumLit = -1;
    let prevCountdownActive = true;
    let puckNearMyGoal = false;
    let nearMissCooldown = 0;

    function handleAudio(state: RS) {
        const myIdx = isHost ? 0 : 1;

        if (state.hit) {
            if (!muted) playHit(state.puck_speed);
            navigator.vibrate?.(12);
        }
        if (state.wall_hit) {
            if (!muted) playWall();
            navigator.vibrate?.(6);
        }
        if (state.goal_scored) {
            const iScored = state.score[myIdx] > prevScore[myIdx];
            if (!muted) iScored ? playMyGoal() : playOpponentGoal();
            navigator.vibrate?.(iScored ? [50, 30, 80] : [40, 30, 40]);
        }

        prevScore = [state.score[0], state.score[1]];

        const inGap = state.puck[0] > GX && state.puck[0] < GX + GOAL_W;
        const myGoalY = isHost ? TH - 12 : 12;
        const nearNow = inGap && (isHost ? state.puck[1] > myGoalY : state.puck[1] < myGoalY);
        nearMissCooldown = Math.max(0, nearMissCooldown - 33);
        if (puckNearMyGoal && !nearNow && !state.goal_scored && nearMissCooldown === 0) {
            if (!muted) playNearMiss();
            nearMissCooldown = 2000;
        }
        puckNearMyGoal = nearNow;

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

        if (!gameOver && (state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE)) {
            const won = state.score[myIdx] >= WINNING_SCORE;
            if (!muted) won ? playWin() : playLose();
        }
    }

    function addBorderPath(g: Graphics) {
        const RC = CR - 2;
        g.moveTo(CR, 2).lineTo(GX, 2);
        g.moveTo(GX + GOAL_W, 2).lineTo(TW - CR, 2);
        g.arc(TW - CR, CR, RC, -Math.PI / 2, 0);
        g.lineTo(TW - 2, TH - CR);
        g.arc(TW - CR, TH - CR, RC, 0, Math.PI / 2);
        g.lineTo(GX + GOAL_W, TH - 2);
        g.moveTo(GX, TH - 2).lineTo(CR, TH - 2);
        g.arc(CR, TH - CR, RC, Math.PI / 2, Math.PI);
        g.lineTo(2, CR);
        g.arc(CR, CR, RC, Math.PI, Math.PI * 1.5);
    }

    function drawBorderGlow(g: Graphics, wf: number) {
        g.clear();
        const idle = 0.06 + (Math.sin(performance.now() * 0.004) + 1) * 0.025;
        addBorderPath(g);
        g.stroke({ width: 6 + wf * 9, color: COL.champagne, alpha: idle + wf * 0.28 });
    }

    function drawBorder(g: Graphics, wf: number) {
        g.clear();
        const t = performance.now() * 0.004;
        const alpha = 0.72 + wf * 0.26;
        const lw = 3.4 + wf * 3.6;

        addBorderPath(g);
        g.stroke({ width: lw + 2.2, color: 0x0a1222, alpha: 0.88 });
        addBorderPath(g);
        g.stroke({ width: lw, color: COL.borderCore, alpha });

        addBorderPath(g);
        g.stroke({ width: 1.2, color: COL.borderAccent, alpha: 0.24 + wf * 0.18 });

        const orbAlpha = 0.35 + (Math.sin(t * 2) + 1) * 0.18 + wf * 0.28;
        g.circle(CR, CR, 4.8).fill({ color: COL.blue, alpha: orbAlpha });
        g.circle(TW - CR, CR, 4.8).fill({ color: COL.green, alpha: orbAlpha });
        g.circle(CR, TH - CR, 4.8).fill({ color: COL.green, alpha: orbAlpha });
        g.circle(TW - CR, TH - CR, 4.8).fill({ color: COL.blue, alpha: orbAlpha });
    }

    function drawBorderRunners(g: Graphics, wf: number) {
        g.clear();
        const t = performance.now() * 0.0038;
        const topLeftSpan = GX - CR;
        const topRightSpan = TW - CR - (GX + GOAL_W);
        const bottomLeftSpan = GX - CR;
        const bottomRightSpan = TW - CR - (GX + GOAL_W);

        const xTopLeft = CR + ((Math.sin(t) + 1) * 0.5) * topLeftSpan;
        const xTopRight = GX + GOAL_W + ((Math.sin(t + 1.6) + 1) * 0.5) * topRightSpan;
        const xBottomRight = GX + GOAL_W + ((Math.sin(t + 3.1) + 1) * 0.5) * bottomRightSpan;
        const xBottomLeft = CR + ((Math.sin(t + 4.5) + 1) * 0.5) * bottomLeftSpan;

        const a = 0.28 + wf * 0.34;
        g.circle(xTopLeft, 2.8, 3.4).fill({ color: COL.blue, alpha: a });
        g.circle(xTopRight, 2.8, 3.4).fill({ color: COL.green, alpha: a });
        g.circle(xBottomRight, TH - 2.8, 3.4).fill({ color: COL.blue, alpha: a });
        g.circle(xBottomLeft, TH - 2.8, 3.4).fill({ color: COL.green, alpha: a });
    }

    function drawGoals(g: Graphics, wf: number) {
        g.clear();
        const wg = wf * 0.34;
        const pulse = 0.08 + (Math.sin(performance.now() * 0.008) + 1) * 0.05;

        g.rect(GX, 0, GOAL_W, 16).fill({ color: 0x34c8a1, alpha: 0.16 + wg + pulse });
        g.moveTo(GX, 0).lineTo(GX, 16).stroke({ width: 2.8, color: 0x34c8a1, alpha: 0.74 + wg });
        g.moveTo(GX + GOAL_W, 0).lineTo(GX + GOAL_W, 16).stroke({ width: 2.8, color: 0x34c8a1, alpha: 0.74 + wg });
        g.rect(GX + 4, 2, GOAL_W - 8, 2.6).fill({ color: 0xc6f8e4, alpha: 0.54 + wg });
        for (let x = GX + 6; x <= GX + GOAL_W - 6; x += 10) {
            g.moveTo(x, 5).lineTo(x + 5, 15).stroke({ width: 0.9, color: 0x85e8c9, alpha: 0.18 + wg });
        }

        g.rect(GX, TH - 16, GOAL_W, 16).fill({ color: 0x5e93de, alpha: 0.16 + wg + pulse });
        g.moveTo(GX, TH).lineTo(GX, TH - 16).stroke({ width: 2.8, color: 0x5e93de, alpha: 0.74 + wg });
        g.moveTo(GX + GOAL_W, TH).lineTo(GX + GOAL_W, TH - 16).stroke({ width: 2.8, color: 0x5e93de, alpha: 0.74 + wg });
        g.rect(GX + 4, TH - 4.6, GOAL_W - 8, 2.6).fill({ color: 0xd3e6ff, alpha: 0.54 + wg });
        for (let x = GX + 6; x <= GX + GOAL_W - 6; x += 10) {
            g.moveTo(x, TH - 15).lineTo(x + 5, TH - 5).stroke({ width: 0.9, color: 0xa7cfff, alpha: 0.18 + wg });
        }
    }

    function drawMidline(g: Graphics, wf: number) {
        g.clear();
        const wave = 0.1 + (Math.sin(performance.now() * 0.01) + 1) * 0.08;
        const alpha = 0.32 + wf * 0.5 + wave;
        const lw = 2.5 + wf * 3;
        for (let x = 0; x < TW; x += 22) {
            const segW = Math.min(14, TW - x);
            const alt = ((x / 22) | 0) % 2 === 0;
            g.rect(x, TH / 2 - lw / 2, segW, lw).fill({ color: alt ? 0xf2dfb1 : 0x7eb5ef, alpha });
        }
    }

    function drawMidlineGlow(g: Graphics, wf: number) {
        g.clear();
        const pulse = 0.05 + (Math.sin(performance.now() * 0.008) + 1) * 0.05;
        const alpha = pulse + wf * 0.3;
        const lw = 8 + wf * 12;
        for (let x = 0; x < TW; x += 22) {
            const segW = Math.min(14, TW - x);
            g.rect(x, TH / 2 - lw / 2, segW, lw).fill({ color: 0xf2dfb1, alpha });
        }
    }

    function drawWallFlashOverlay(g: Graphics, wf: number) {
        g.clear();
        if (wf <= 0.01) return;
        g.rect(0, 0, TW, TH).fill({ color: 0xffffff, alpha: wf * 0.06 });
    }

    function drawTrailGfx(g: Graphics, trailData: typeof trail) {
        g.clear();
        for (let i = trailData.length - 1; i >= 0; i--) {
            const t = trailData[i];
            const a = (1 - t.age / 0.1) * 0.55;
            const r = PR * (1 - t.age / 0.1) * 0.75;
            g.circle(t.x, t.y, r).fill({ color: COL.puck, alpha: a });
        }
    }

    function drawPuckFx(g: Graphics, px: number, py: number, speedGain: number, now: number) {
        g.clear();
        const pulse = 0.5 + (Math.sin(now * 0.02) + 1) * 0.25;
        const ringR = PR * 0.82;
        g.circle(px, py, ringR).stroke({ width: 1.6 + speedGain * 0.8, color: 0xf2f6fb, alpha: 0.28 + speedGain * 0.2 });
        g.circle(px, py, PR * 0.48 + pulse * 1.2).fill({ color: 0xf8fbff, alpha: 0.14 + speedGain * 0.12 });
        g.circle(px, py, PR * 0.3 + pulse * 0.7).stroke({ width: 1.1, color: 0xe6edf5, alpha: 0.4 + speedGain * 0.1 });

        const orbit = PR * 0.64;
        const a = now * 0.012;
        g.circle(px + Math.cos(a) * orbit, py + Math.sin(a) * orbit, 1.8)
            .fill({ color: 0x9cc9f2, alpha: 0.36 + speedGain * 0.2 });
        g.circle(px + Math.cos(a + Math.PI) * orbit, py + Math.sin(a + Math.PI) * orbit, 1.6)
            .fill({ color: 0x4ca0ea, alpha: 0.32 + speedGain * 0.16 });
    }

    function drawPuckDecal(g: Graphics, px: number, py: number, now: number) {
        g.clear();
        const r = PR * 0.58;
        const a = now * 0.006;
        const dx = Math.cos(a) * r;
        const dy = Math.sin(a) * r;

        // Strong split-color dots make puck read as non-mono at a glance.
        g.circle(px + dx, py + dy, PR * 0.12).fill({ color: 0x60a5fa, alpha: 0.88 });
        g.circle(px - dx, py - dy, PR * 0.12).fill({ color: 0x34d399, alpha: 0.88 });

        // Crosshair ring keeps structure visible under motion blur.
        g.circle(px, py, PR * 0.36).stroke({ width: 1.2, color: 0xffffff, alpha: 0.72 });
        g.moveTo(px - PR * 0.2, py).lineTo(px + PR * 0.2, py).stroke({ width: 1.1, color: 0x111827, alpha: 0.8 });
        g.moveTo(px, py - PR * 0.2).lineTo(px, py + PR * 0.2).stroke({ width: 1.1, color: 0x111827, alpha: 0.8 });
    }

    function drawPaddleFx(
        g: Graphics,
        blue: [number, number],
        green: [number, number],
        now: number,
        wallFlash: number,
    ) {
        g.clear();
        const pulse = 0.5 + (Math.sin(now * 0.016) + 1) * 0.25;
        const ringR = PAR + 6.5 + pulse * 1.8;

        g.circle(blue[0], blue[1], ringR).stroke({ width: 2, color: 0x60a5fa, alpha: 0.28 + wallFlash * 0.24 });
        g.circle(blue[0], blue[1], PAR * 0.56).fill({ color: 0x93c5fd, alpha: 0.12 + wallFlash * 0.08 });

        g.circle(green[0], green[1], ringR).stroke({ width: 2, color: 0x34d399, alpha: 0.28 + wallFlash * 0.24 });
        g.circle(green[0], green[1], PAR * 0.56).fill({ color: 0x6ee7b7, alpha: 0.12 + wallFlash * 0.08 });
    }

    function drawPaddleDecals(g: Graphics, blue: [number, number], green: [number, number], now: number) {
        g.clear();
        const a = now * 0.004;
        const pulse = 0.88 + (Math.sin(now * 0.01) + 1) * 0.06;

        const drawFor = (x: number, y: number, c1: number, c2: number) => {
            // Two opposite colored tabs + inner contrast ring.
            const ox = Math.cos(a) * PAR * 0.62;
            const oy = Math.sin(a) * PAR * 0.62;
            g.circle(x + ox, y + oy, PAR * 0.16).fill({ color: c1, alpha: 0.95 });
            g.circle(x - ox, y - oy, PAR * 0.16).fill({ color: c2, alpha: 0.95 });

            g.circle(x, y, PAR * 0.46).stroke({ width: 1.5, color: 0xffffff, alpha: 0.75 * pulse });
            g.circle(x, y, PAR * 0.22).fill({ color: 0x0b1220, alpha: 0.55 });
        };

        drawFor(blue[0], blue[1], 0x93c5fd, 0x2563eb);
        drawFor(green[0], green[1], 0x6ee7b7, 0x059669);
    }

    function drawActorFallback(
        g: Graphics,
        puck: [number, number],
        blue: [number, number],
        green: [number, number],
        speedGain: number,
    ) {
        g.clear();

        // Safety layer used only when advanced rendering paths are disabled.
        g.circle(blue[0], blue[1], PAR * 0.9).fill({ color: 0x60a5fa, alpha: 0.36 });
        g.circle(blue[0], blue[1], PAR * 0.9).stroke({ width: 2.2, color: 0xdbeafe, alpha: 0.9 });

        g.circle(green[0], green[1], PAR * 0.9).fill({ color: 0x34d399, alpha: 0.36 });
        g.circle(green[0], green[1], PAR * 0.9).stroke({ width: 2.2, color: 0xd1fae5, alpha: 0.9 });

        g.circle(puck[0], puck[1], PR * 0.86).fill({ color: 0xf59e0b, alpha: 0.52 + speedGain * 0.18 });
        g.circle(puck[0], puck[1], PR * 0.86).stroke({ width: 2, color: 0xfff7c2, alpha: 0.95 });
    }

    function updateScores() {
        const myIdx = isHost ? 0 : 1;
        const opIdx = isHost ? 1 : 0;
        const myCol = isHost ? "#60a5fa" : "#34d399";
        const opCol = isHost ? "#56d8b0" : "#6eaeea";

        const opSz = 72 + rs.score_flash[opIdx] * 28;
        const mySz = 72 + rs.score_flash[myIdx] * 28;

        scoreGlowGfx.clear();
        if (rs.score_flash[opIdx] > 0.01) {
            scoreGlowGfx.circle(54, pxH / 2 - 44, 52 + rs.score_flash[opIdx] * 24).fill({
                color: isHost ? 0x34d399 : 0x60a5fa,
                alpha: 0.12 + rs.score_flash[opIdx] * 0.3,
            });
        }
        if (rs.score_flash[myIdx] > 0.01) {
            scoreGlowGfx.circle(54, pxH / 2 + 52, 52 + rs.score_flash[myIdx] * 24).fill({
                color: isHost ? 0x60a5fa : 0x34d399,
                alpha: 0.12 + rs.score_flash[myIdx] * 0.3,
            });
        }

        opScoreText.text = String(rs.score[opIdx]);
        opScoreText.style.fontSize = opSz;
        opScoreText.style.fill = opCol;
        opScoreText.alpha = 0.5 + rs.score_flash[opIdx] * 0.5;
        opScoreText.position.set(20, pxH / 2 - 8);

        const opM = opScoreText.getLocalBounds();
        opScoreBg.clear();
        opScoreBg.roundRect(10, pxH / 2 - 8 - opSz - 12, opM.width + 34, opSz + 20, 18).fill({ color: 0x02060d, alpha: 0.42 });
        opScoreBg.roundRect(10, pxH / 2 - 8 - opSz - 12, opM.width + 34, opSz + 20, 18).stroke({ width: 1.5, color: isHost ? 0x4ad9aa : 0x76c8ff, alpha: 0.34 });
        opScoreBg.rect(20, pxH / 2 - 8 - opSz + opSz + 1, Math.max(16, opM.width + 14), 2.4).fill({ color: isHost ? 0x4ad9aa : 0x76c8ff, alpha: 0.45 });

        myScoreText.text = String(rs.score[myIdx]);
        myScoreText.style.fontSize = mySz;
        myScoreText.style.fill = myCol;
        myScoreText.alpha = 0.85 + rs.score_flash[myIdx] * 0.15;
        myScoreText.position.set(20, pxH / 2 + 8);

        const myM = myScoreText.getLocalBounds();
        myScoreBg.clear();
        myScoreBg.roundRect(10, pxH / 2, myM.width + 34, mySz + 20, 18).fill({ color: 0x02060d, alpha: 0.42 });
        myScoreBg.roundRect(10, pxH / 2, myM.width + 34, mySz + 20, 18).stroke({ width: 1.5, color: isHost ? 0x76c8ff : 0x4ad9aa, alpha: 0.34 });
        myScoreBg.rect(20, pxH / 2 + mySz + 7, Math.max(16, myM.width + 14), 2.4).fill({ color: isHost ? 0x76c8ff : 0x4ad9aa, alpha: 0.45 });
    }

    function updateCountdown() {
        if (rs.countdown <= 0) {
            countdownContainer.visible = false;
            return;
        }

        countdownContainer.visible = true;
        const numLit = Math.min(5, Math.floor((3.0 - rs.countdown) / 0.5));
        const LIGHTS = 5;
        const R = Math.round(pxW * 0.055);
        const GAP = Math.round(R * 0.55);
        const totalW = LIGHTS * 2 * R + (LIGHTS - 1) * GAP;
        const lx0 = pxW / 2 - totalW / 2 + R;
        const ly = pxH * 0.42;

        const pw = totalW + R * 1.6;
        const ph = R * 2.8;
        countdownPanel.clear();
        countdownPanel.roundRect(pxW / 2 - pw / 2, ly - ph / 2, pw, ph, R * 0.35).fill({ color: 0x050d1b, alpha: 0.8 });
        countdownPanel.roundRect(pxW / 2 - pw / 2, ly - ph / 2, pw, ph, R * 0.35).stroke({ width: 1.2, color: 0x27405f, alpha: 0.6 });
        countdownPanel.rect(pxW / 2 - pw / 2, ly - 2, pw, 4).fill({ color: 0x11233d, alpha: 0.9 });

        const lightSpriteSize = Math.ceil(R * 5);
        for (let i = 0; i < LIGHTS; i++) {
            const spr = countdownLightSprites[i];
            const lit = i < numLit;
            spr.texture = lit ? litLightTex : unlitLightTex;
            spr.width = lightSpriteSize;
            spr.height = lightSpriteSize;
            spr.position.set(lx0 + i * (2 * R + GAP), ly);
            spr.alpha = lit ? 1 : 0.96;
            const glowSpr = countdownLightSprites[i + LIGHTS];
            glowSpr.texture = litLightGlowTex;
            glowSpr.width = lightSpriteSize;
            glowSpr.height = lightSpriteSize;
            glowSpr.position.set(lx0 + i * (2 * R + GAP), ly);
            glowSpr.alpha = lit ? 0.8 : 0;
        }
    }

    function update() {
        if (!app) return;

        if (pendingPtr) {
            const [x, y] = pendingPtr;
            pendingPtr = null;
            invoke("set_pointer", { x, y }).catch(() => {});
        }

        const now = performance.now();
        const dt = lastTime ? Math.min((now - lastTime) / 1000, 0.05) : 0;
        lastTime = now;

        const dx = rs.puck[0] - lastPuckX;
        const dy = rs.puck[1] - lastPuckY;
        if (dx * dx + dy * dy > 4) {
            trail.unshift({ x: rs.puck[0], y: rs.puck[1], age: 0 });
            lastPuckX = rs.puck[0];
            lastPuckY = rs.puck[1];
        }
        for (const t of trail) t.age += dt;
        trail = trail.filter((t) => t.age < 0.1).slice(0, 6);

        if (!isHost && !isSinglePlayer) {
            tableContainer.position.set(ox + TW * scale, oy + TH * scale);
            tableContainer.scale.set(-scale, -scale);
        } else {
            tableContainer.position.set(ox, oy);
            tableContainer.scale.set(scale, scale);
        }

        if (tableFxEnabled) {
            try {
            drawBorderGlow(borderGlowGfx, rs.wall_flash);
            drawBorderRunners(borderRunnerGfx, rs.wall_flash);
            drawMidlineGlow(midlineGlowGfx, rs.wall_flash);
            drawWallFlashOverlay(wallFlashGfx, rs.wall_flash);
            drawBorder(borderGfx, rs.wall_flash);
            drawGoals(goalsGfx, rs.wall_flash);
            drawMidline(midlineGfx, rs.wall_flash);
            drawTrailGfx(trailGfx, trail);
            } catch (e) {
                // If a device cannot handle one FX path, keep gameplay visible by disabling advanced table FX.
                tableFxEnabled = false;
                actorFallbackEnabled = true;
                console.error("pixi table FX disabled after draw failure", e);
                borderGlowGfx.clear();
                borderRunnerGfx.clear();
                midlineGlowGfx.clear();
                wallFlashGfx.clear();
                borderGfx.clear();
                goalsGfx.clear();
                midlineGfx.clear();
                trailGfx.clear();
                puckFxGfx.clear();
                paddleFxGfx.clear();
            }
        }

        const sg = Math.min(rs.puck_speed / MAX_SPEED, 1);
        if (tableFxEnabled) {
            drawPuckFx(puckFxGfx, rs.puck[0], rs.puck[1], sg, now);
            drawPuckDecal(puckDecalGfx, rs.puck[0], rs.puck[1], now);
        }
        puckBodySpr.position.set(rs.puck[0], rs.puck[1]);
        puckBodySpr.rotation += 0.02 + sg * 0.07;
        puckIconSpr.position.set(rs.puck[0], rs.puck[1]);
        puckIconSpr.rotation -= 0.016 + sg * 0.05;
        puckIconSpr.alpha = 0.9;

        const myPos = localPaddlePos ?? (isHost ? rs.host_paddle : rs.client_paddle);
        const oppPos = isHost ? rs.client_paddle : rs.host_paddle;
        const bluePos = isHost ? myPos : oppPos;
        const greenPos = isHost ? oppPos : myPos;
        if (tableFxEnabled) {
            drawPaddleFx(paddleFxGfx, bluePos, greenPos, now, rs.wall_flash);
            drawPaddleDecals(paddleDecalGfx, bluePos, greenPos, now);
        }
        const needsActorFallback =
            actorFallbackEnabled
            && (puckBodySpr.texture.width <= 0 || paddleBlueSpr.texture.width <= 0 || paddleGreenSpr.texture.width <= 0);
        if (needsActorFallback) {
            drawActorFallback(actorFallbackGfx, rs.puck, bluePos, greenPos, sg);
        } else {
            actorFallbackGfx.clear();
        }
        paddleBlueSpr.position.set(bluePos[0], bluePos[1]);
        paddleGreenSpr.position.set(greenPos[0], greenPos[1]);
        const padPulse = 1 + (Math.sin(now * 0.01) + 1) * 0.012;
        paddleBlueSpr.scale.set(padPulse);
        paddleGreenSpr.scale.set(padPulse);
        paddleBlueIconSpr.position.set(bluePos[0], bluePos[1]);
        paddleGreenIconSpr.position.set(greenPos[0], greenPos[1]);
        paddleBlueIconSpr.rotation += 0.01;
        paddleGreenIconSpr.rotation -= 0.01;
        paddleBlueIconSpr.scale.set(padPulse);
        paddleGreenIconSpr.scale.set(padPulse);

        if (rs.goal_flash > 0) {
            goalFlashGfx.visible = true;
            goalFlashGfx.clear();
            goalFlashGfx.rect(0, 0, TW, TH).fill({ color: 0xffffff, alpha: rs.goal_flash * 0.25 });
        } else {
            goalFlashGfx.visible = false;
        }

        updateScores();
        updateCountdown();
    }

    function tableCoords(cx: number, cy: number): [number, number] {
        let x = (cx * dpr - ox) / scale;
        let y = (cy * dpr - oy) / scale;
        if (!isHost && !isSinglePlayer) {
            x = TW - x;
            y = TH - y;
        }
        return [x, y];
    }

    function onPointerMove(e: PointerEvent) {
        e.preventDefault();
        unlockAudioOnce();
        const [rawX, rawY] = tableCoords(e.clientX, e.clientY);

        // Match Rust-side paddle clamps so local prediction never escapes playable bounds.
        const x = Math.min(Math.max(rawX, PAR), TW - PAR);
        const y = isHost
            ? Math.min(Math.max(rawY, TH / 2.0 + PAR / 2.0), TH - PAR)
            : Math.min(Math.max(rawY, PAR), TH / 2.0 - PAR / 2.0);

        pendingPtr = [x, y];
        localPaddlePos = [x, y];
    }

    function onResize() {
        if (!app) return;
        dpr = Math.min(window.devicePixelRatio || 1, 2);
        pxW = window.innerWidth * dpr;
        pxH = window.innerHeight * dpr;
        app.renderer.resize(pxW, pxH);
        app.canvas.style.width = window.innerWidth + "px";
        app.canvas.style.height = window.innerHeight + "px";
        scale = Math.min(pxW / TW, pxH / TH) * 0.92;
        ox = (pxW - TW * scale) / 2;
        oy = (pxH - TH * scale) / 2;

        const R = Math.round(pxW * 0.055);
        litLightTex = buildLightTexture(true, R);
        unlitLightTex = buildLightTexture(false, R);
        litLightGlowTex = buildLightGlowTexture(R);
    }

    async function rematch() {
        gameOver = false;
        prevScore = [0, 0];
        prevNumLit = -1;
        prevCountdownActive = true;
        puckNearMyGoal = false;
        nearMissCooldown = 0;
        rs = {
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

    onMount(async () => {
        preloadAd();

        dpr = Math.min(window.devicePixelRatio || 1, 2);
        pxW = window.innerWidth * dpr;
        pxH = window.innerHeight * dpr;

        app = new Application();
        await app.init({
            width: pxW,
            height: pxH,
            resolution: 1,
            autoDensity: false,
            backgroundColor: 0x060b14,
            antialias: true,
            powerPreference: "high-performance",
        });

        app.canvas.style.width = window.innerWidth + "px";
        app.canvas.style.height = window.innerHeight + "px";
        app.canvas.style.touchAction = "none";
        containerEl.appendChild(app.canvas);
        mountedCanvas = app.canvas;

        scale = Math.min(pxW / TW, pxH / TH) * 0.92;
        ox = (pxW - TW * scale) / 2;
        oy = (pxH - TH * scale) / 2;

        const staticBgTex = buildStaticBgTexture();
        const paddleBlueTex = buildPaddleTexture("#3b82f6", "#93c5fd");
        const paddleGreenTex = buildPaddleTexture("#10b981", "#6ee7b7");
        const puckBodyTex = buildPuckBodyTexture();
        const puckIconTex = buildPuckIconTexture();
        const paddleBlueIconTex = buildPaddleIconTexture("#2563eb");
        const paddleGreenIconTex = buildPaddleIconTexture("#059669");
        const R = Math.round(pxW * 0.055);
        litLightTex = buildLightTexture(true, R);
        unlitLightTex = buildLightTexture(false, R);
        litLightGlowTex = buildLightGlowTexture(R);

        tableContainer = new Container();
        app.stage.addChild(tableContainer);

        const staticBgSprite = new Sprite(staticBgTex);
        tableContainer.addChild(staticBgSprite);

        borderGlowGfx = new Graphics();
        tableContainer.addChild(borderGlowGfx);

        midlineGlowGfx = new Graphics();
        tableContainer.addChild(midlineGlowGfx);

        wallFlashGfx = new Graphics();
        tableContainer.addChild(wallFlashGfx);

        borderRunnerGfx = new Graphics();
        tableContainer.addChild(borderRunnerGfx);

        borderGfx = new Graphics();
        tableContainer.addChild(borderGfx);

        goalsGfx = new Graphics();
        tableContainer.addChild(goalsGfx);

        midlineGfx = new Graphics();
        tableContainer.addChild(midlineGfx);

        trailGfx = new Graphics();
        tableContainer.addChild(trailGfx);

        paddleFxGfx = new Graphics();
        tableContainer.addChild(paddleFxGfx);

        paddleDecalGfx = new Graphics();
        tableContainer.addChild(paddleDecalGfx);

        puckFxGfx = new Graphics();
        tableContainer.addChild(puckFxGfx);

        puckDecalGfx = new Graphics();
        tableContainer.addChild(puckDecalGfx);

        actorFallbackGfx = new Graphics();
        tableContainer.addChild(actorFallbackGfx);

        puckBodySpr = new Sprite(puckBodyTex);
        puckBodySpr.anchor.set(0.5);
        tableContainer.addChild(puckBodySpr);

        puckIconSpr = new Sprite(puckIconTex);
        puckIconSpr.anchor.set(0.5);
        tableContainer.addChild(puckIconSpr);

        paddleBlueSpr = new Sprite(paddleBlueTex);
        paddleBlueSpr.anchor.set(0.5);
        tableContainer.addChild(paddleBlueSpr);

        paddleBlueIconSpr = new Sprite(paddleBlueIconTex);
        paddleBlueIconSpr.anchor.set(0.5);
        tableContainer.addChild(paddleBlueIconSpr);

        paddleGreenSpr = new Sprite(paddleGreenTex);
        paddleGreenSpr.anchor.set(0.5);
        tableContainer.addChild(paddleGreenSpr);

        paddleGreenIconSpr = new Sprite(paddleGreenIconTex);
        paddleGreenIconSpr.anchor.set(0.5);
        tableContainer.addChild(paddleGreenIconSpr);

        goalFlashGfx = new Graphics();
        goalFlashGfx.visible = false;
        tableContainer.addChild(goalFlashGfx);

        hudContainer = new Container();
        app.stage.addChild(hudContainer);

        scoreGlowGfx = new Graphics();
        hudContainer.addChild(scoreGlowGfx);

        opScoreBg = new Graphics();
        hudContainer.addChild(opScoreBg);
        opScoreText = new Text({
            text: "0",
            style: new TextStyle({
                fontFamily: "Space Grotesk, Sora, system-ui",
                fontWeight: "900",
                fontSize: 72,
                fill: "#34d399",
            }),
        });
        opScoreText.anchor.set(0, 1);
        hudContainer.addChild(opScoreText);

        myScoreBg = new Graphics();
        hudContainer.addChild(myScoreBg);
        myScoreText = new Text({
            text: "0",
            style: new TextStyle({
                fontFamily: "Space Grotesk, Sora, system-ui",
                fontWeight: "900",
                fontSize: 72,
                fill: "#60a5fa",
            }),
        });
        myScoreText.anchor.set(0, 0);
        hudContainer.addChild(myScoreText);

        countdownContainer = new Container();
        hudContainer.addChild(countdownContainer);
        countdownPanel = new Graphics();
        countdownContainer.addChild(countdownPanel);

        for (let i = 0; i < 5; i++) {
            const spr = new Sprite(unlitLightTex);
            spr.anchor.set(0.5);
            countdownLightSprites.push(spr);
            countdownContainer.addChild(spr);
        }
        for (let i = 0; i < 5; i++) {
            const glowSpr = new Sprite(litLightGlowTex);
            glowSpr.anchor.set(0.5);
            glowSpr.alpha = 0;
            countdownLightSprites.push(glowSpr);
            countdownContainer.addChild(glowSpr);
        }

        app.canvas.addEventListener("pointermove", onPointerMove, { passive: false });
        app.canvas.addEventListener("pointerdown", onPointerMove, { passive: false });
        window.addEventListener("resize", onResize);

        onVisibility = () => {
            if (document.hidden) invoke("pause_game").catch(() => {});
            else invoke("resume_game").catch(() => {});
        };
        document.addEventListener("visibilitychange", onVisibility);

        app.ticker.add(update);

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

        await invoke("stop_game").catch(() => {});

        try {
            await invoke("start_game", { isHost, isSinglePlayer, channel: ch, useUdp });
        } catch (e) {
            console.error("start_game failed:", e);
            onBack?.();
            return;
        }
    });

    onDestroy(async () => {
        if (app) {
            app.ticker.remove(update);
        }
        if (mountedCanvas) {
            mountedCanvas.removeEventListener("pointermove", onPointerMove);
            mountedCanvas.removeEventListener("pointerdown", onPointerMove);
        }
        window.removeEventListener("resize", onResize);
        if (onVisibility) {
            document.removeEventListener("visibilitychange", onVisibility);
        }

        if (app) {
            app.destroy(true);
            app = null;
        }

        await invoke("stop_game").catch(() => {});
    });
</script>

<div
    bind:this={containerEl}
    class="touch-none block"
    style="width: 100vw; height: 100vh; position: fixed; top: 0; left: 0; background: radial-gradient(circle at 50% 35%, #0d1f3d 0%, #050d1d 58%, #030814 100%);"
></div>

{#if gameOver}
<div class="fixed inset-0 flex flex-col items-center justify-center z-10 bg-black/80 backdrop-blur-md">
    <div class="flex flex-col items-center gap-6 p-12 rounded-[2.5rem] bg-gradient-to-br from-neutral-900/95 to-neutral-800/90 border border-neutral-600/50 shadow-[0_0_60px_rgba(0,0,0,0.6)] animate-in fade-in zoom-in duration-300">
        <div class="text-7xl {iWon ? 'animate-bounce' : 'animate-pulse'}">{iWon ? '🏆' : '😔'}</div>
        <h2 class="text-5xl font-black {iWon ? 'text-yellow-400 drop-shadow-[0_0_24px_rgba(250,204,21,0.6)]' : 'text-neutral-400'}">{iWon ? 'VICTORY!' : 'DEFEAT'}</h2>
        <div class="flex items-center gap-4 text-4xl font-black text-white tracking-widest bg-neutral-800/50 px-8 py-4 rounded-2xl border border-neutral-600/30">
            <span class="{rs.score[0] > rs.score[1] ? 'text-yellow-400' : 'text-neutral-400'}">{rs.score[0]}</span>
            <span class="text-neutral-600">-</span>
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
