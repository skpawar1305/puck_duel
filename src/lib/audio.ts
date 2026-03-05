// Global AudioContext wrapper to allow initialization from safe DOM click events
export let audioCtx: AudioContext | null = null;

export function initAudio() {
    if (!audioCtx) {
        audioCtx = new AudioContext();
    }
    if (audioCtx.state === "suspended") {
        audioCtx.resume();
    }
}

function note(type: OscillatorType, freq: number, vol: number, dur: number, at: number, freqEnd?: number) {
    if (!audioCtx) return;
    const o = audioCtx.createOscillator();
    const g = audioCtx.createGain();
    o.connect(g); g.connect(audioCtx.destination);
    o.type = type;
    o.frequency.setValueAtTime(freq, at);
    if (freqEnd) o.frequency.exponentialRampToValueAtTime(freqEnd, at + dur);
    g.gain.setValueAtTime(vol, at);
    g.gain.exponentialRampToValueAtTime(0.001, at + dur);
    o.start(at); o.stop(at + dur + 0.01);
}

// Paddle hit — pitch and crunch scale with puck speed
export function playHit(speed = 300) {
    if (!audioCtx) return;
    const t = audioCtx.currentTime;
    const n = Math.min(speed / 600, 1);
    note("sine", 320 + n * 480, 0.35 + n * 0.45, 0.09 + n * 0.04, t, 70 + n * 30);
    // Hard-hit crack overtone
    if (n > 0.35) note("sawtooth", 900 + n * 600, (n - 0.35) * 0.32, 0.03, t, 150);
}

// Wall bounce — unchanged feel, slightly shorter
export function playWall() {
    if (!audioCtx) return;
    note("square", 260, 0.3, 0.07, audioCtx.currentTime, 120);
}

// I scored — ascending Cmaj7 arpeggio + bass thump
export function playMyGoal() {
    if (!audioCtx) return;
    const ctx = audioCtx, t = ctx.currentTime;
    // Bass thump
    note("sine", 110, 0.7, 0.2, t, 55);
    // Arpeggio C4 E4 G4 B4 C5 E5
    [262, 330, 392, 494, 523, 659].forEach((freq, i) => {
        note(i < 4 ? "sine" : "triangle", freq, 0.35, 0.22, t + i * 0.1);
    });
}

// Opponent scored — short descending minor motif
export function playOpponentGoal() {
    if (!audioCtx) return;
    const t = audioCtx.currentTime;
    [392, 311].forEach((freq, i) => {
        note("sine", freq, 0.22, 0.17, t + i * 0.15, freq * 0.85);
    });
}

// Puck came very close to my goal but didn't go in — tension whoosh + relief note
export function playNearMiss() {
    if (!audioCtx) return;
    const ctx = audioCtx, t = ctx.currentTime;
    // Descending tension whoosh
    const o = ctx.createOscillator(), f = ctx.createBiquadFilter(), g = ctx.createGain();
    o.type = "sawtooth"; o.frequency.setValueAtTime(800, t);
    o.frequency.exponentialRampToValueAtTime(90, t + 0.22);
    f.type = "bandpass"; f.frequency.value = 350; f.Q.value = 0.6;
    o.connect(f); f.connect(g); g.connect(ctx.destination);
    g.gain.setValueAtTime(0.22, t); g.gain.exponentialRampToValueAtTime(0.001, t + 0.24);
    o.start(t); o.stop(t + 0.25);
    // Soft "phew" rising relief tone after the danger passes
    const o2 = ctx.createOscillator(), g2 = ctx.createGain();
    o2.type = "sine"; o2.frequency.setValueAtTime(330, t + 0.18);
    o2.frequency.linearRampToValueAtTime(440, t + 0.42);
    g2.gain.setValueAtTime(0.0001, t + 0.18);
    g2.gain.linearRampToValueAtTime(0.18, t + 0.26);
    g2.gain.exponentialRampToValueAtTime(0.001, t + 0.44);
    o2.connect(g2); g2.connect(ctx.destination);
    o2.start(t + 0.18); o2.stop(t + 0.45);
}

// F1 light click — each lamp illuminating
export function playCountdownTick() {
    if (!audioCtx) return;
    note("square", 180, 0.22, 0.025, audioCtx.currentTime, 160);
}

// Lights out — GO! Rising sweep + chord burst
export function playCountdownGo() {
    if (!audioCtx) return;
    const ctx = audioCtx, t = ctx.currentTime;
    note("sawtooth", 180, 0.14, 0.32, t, 820);
    [523, 659, 784].forEach((freq, i) => note("sine", freq, 0.28, 0.4, t + 0.28 + i * 0.04));
}

// Win fanfare — C major scale run + final chord
export function playWin() {
    if (!audioCtx) return;
    const ctx = audioCtx, t = ctx.currentTime;
    [262, 294, 330, 349, 392, 440, 494, 523].forEach((freq, i) => {
        note("sine", freq, 0.28, 0.16, t + i * 0.07);
    });
    [262, 330, 392, 523].forEach(freq => note("triangle", freq, 0.32, 0.75, t + 0.66));
}

// Lose — descending sad trombone
export function playLose() {
    if (!audioCtx) return;
    const t = audioCtx.currentTime;
    [523, 466, 415, 392].forEach((freq, i) => {
        note("sawtooth", freq, 0.22, 0.2, t + i * 0.19, freq * 0.94);
    });
}
