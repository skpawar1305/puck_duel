use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use crate::transport::TransportState;


const TW: f32 = 360.0;
const TH: f32 = 640.0;
const PR: f32 = 20.0;
const PAR: f32 = 27.0;
const GOAL_W: f32 = 110.0;
const GX: f32 = (TW - GOAL_W) / 2.0;
const CR: f32 = 42.0;
const MAX_SPEED: f32 = 990.0;
const WALL_REST: f32 = 0.88;
const FRICTION: f32 = 0.22;

// ── Public state managed by Tauri ─────────────────────────────────────────────
pub struct GameEngine {
    pub running: Arc<AtomicBool>,
    pub paused:  Arc<AtomicBool>,
    pub pointer: Arc<Mutex<[f32; 2]>>,
}
impl GameEngine {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            paused:  Arc::new(AtomicBool::new(false)),
            pointer: Arc::new(Mutex::new([TW / 2.0, TH - 120.0])),
        }
    }
}

// ── Render state sent to JS via Channel every frame ───────────────────────────
#[derive(serde::Serialize, Clone)]
pub struct RenderState {
    pub puck:          [f32; 2],
    pub puck_speed:    f32,
    pub host_paddle:   [f32; 2],
    pub client_paddle: [f32; 2],
    pub score:         [u32; 2],
    pub wall_flash:    f32,
    pub goal_flash:    f32,
    pub score_flash:   [f32; 2],
    pub hit:           u8,   // 1 = paddle hit this frame
    pub wall_hit:      u8,   // 1 = wall hit this frame
    pub goal_scored:   u8,   // 1 = goal this frame
    pub countdown:     f32,  // >0 = pre-game countdown
}

// ── Internal physics structs ──────────────────────────────────────────────────
#[derive(Clone)]
struct Puck  { x: f32, y: f32, vx: f32, vy: f32 }
#[derive(Clone)]
struct Paddle{ x: f32, y: f32, pvx: f32, pvy: f32 }

// ── Physics helpers ───────────────────────────────────────────────────────────
fn collide_paddle_puck(puck: &mut Puck, pad: &Paddle) -> bool {
    let dx = puck.x - pad.x;
    let dy = puck.y - pad.y;
    let d  = (dx*dx + dy*dy).sqrt();
    let md = PR + PAR;
    if d >= md || d < 0.001 { return false; }
    let (nx, ny) = (dx/d, dy/d);
    puck.x = pad.x + nx * (md + 1.0);
    puck.y = pad.y + ny * (md + 1.0);
    let dot = (puck.vx - pad.pvx)*nx + (puck.vy - pad.pvy)*ny;
    if dot < 0.0 { puck.vx -= 2.0*dot*nx; puck.vy -= 2.0*dot*ny; true } else { false }
}

fn collide_corner_puck(puck: &mut Puck, cx: f32, cy: f32, in_zone: bool) -> bool {
    if !in_zone { return false; }
    let dx = puck.x - cx;
    let dy = puck.y - cy;
    let d  = (dx*dx + dy*dy).sqrt();
    let max_d = (CR - 2.0) - PR;
    if d <= max_d || d < 0.001 { return false; }
    let (nx, ny) = (dx/d, dy/d);
    puck.x = cx + nx * max_d;
    puck.y = cy + ny * max_d;
    let dot = puck.vx*nx + puck.vy*ny;
    if dot > 0.0 { puck.vx -= dot*(1.0+WALL_REST)*nx; puck.vy -= dot*(1.0+WALL_REST)*ny; true } else { false }
}

fn collide_goal_post(puck: &mut Puck, px: f32, py: f32) -> bool {
    let dx = puck.x - px;
    let dy = puck.y - py;
    let d  = (dx*dx + dy*dy).sqrt();
    if d >= PR || d < 0.001 { return false; }
    let (nx, ny) = (dx/d, dy/d);
    puck.x = px + nx * PR;
    puck.y = py + ny * PR;
    let dot = puck.vx*nx + puck.vy*ny;
    if dot < 0.0 { puck.vx -= (1.0+WALL_REST)*dot*nx; puck.vy -= (1.0+WALL_REST)*dot*ny; true } else { false }
}

// ── Game state ────────────────────────────────────────────────────────────────
struct GameState {
    puck:            Puck,
    host_paddle:     Paddle,
    client_paddle:   Paddle,
    score:           [u32; 2],

    // dead-reckoning targets (non-auth side)
    target_puck:     Puck,
    target_opponent: [f32; 2],

    // visual FX
    wall_flash:      f32,
    goal_flash:      f32,
    score_flash:     [f32; 2],

    // audio cues consumed by renderer
    hit:        u8,
    wall_hit:   u8,
    goal_scored:u8,
    countdown:  f32,

    // authority tracking
    prev_auth:           bool,
    prev_recv_host_auth: bool, // last received isHostAuth — detect handoff transitions
    is_host:             bool,
    is_single:           bool,
}

impl GameState {
    fn new(is_host: bool, is_single: bool) -> Self {
        let (yh, yc) = (TH - 120.0, 120.0);
        Self {
            puck:            Puck  { x:TW/2.0, y:TH/2.0, vx:0.0, vy:0.0 },
            host_paddle:     Paddle{ x:TW/2.0, y:yh, pvx:0.0, pvy:0.0 },
            client_paddle:   Paddle{ x:TW/2.0, y:yc, pvx:0.0, pvy:0.0 },
            score:           [0, 0],
            target_puck:     Puck  { x:TW/2.0, y:TH/2.0, vx:0.0, vy:0.0 },
            target_opponent: if is_host { [TW/2.0, yc] } else { [TW/2.0, yh] },
            wall_flash:0.0, goal_flash:0.0, score_flash:[0.0,0.0],
            hit:0, wall_hit:0, goal_scored:0, countdown: 3.0,
            prev_auth: is_single || is_host,
            prev_recv_host_auth: true, // host starts authoritative
            is_host, is_single,
        }
    }

    fn auth(&self) -> bool {
        if self.is_single { return true; }
        // Use target_puck.y (network-ground-truth): ensures exactly ONE side is
        // authoritative at all times. On auth frames we echo target_puck = puck,
        // so this is always fresh. On non-auth frames it reflects the peer's last
        // sent position — the definitive record of which half the puck is in.
        let y = self.target_puck.y;
        if self.is_host { y >= TH / 2.0 }
        else            { y <  TH / 2.0 }
    }

    fn reset_puck(&mut self) {
        self.puck = Puck { x:TW/2.0, y:TH/2.0, vx:0.0, vy:0.0 };
    }

    fn update(&mut self, dt: f32, ptr: [f32; 2]) {
        self.hit = 0; self.wall_hit = 0; self.goal_scored = 0;

        // Countdown — paddles move but puck stays frozen
        if self.countdown > 0.0 {
            self.countdown = (self.countdown - dt).max(0.0);
        }

        // ── My paddle ────────────────────────────────────────────────────────
        {
            let p = if self.is_host { &mut self.host_paddle } else { &mut self.client_paddle };
            let (px, py) = (p.x, p.y);

            p.x = ptr[0].max(PAR).min(TW - PAR);
            if self.is_host { p.y = ptr[1].max(TH / 2.0 + PAR / 2.0).min(TH - PAR); }
            else             { p.y = ptr[1].max(PAR).min(TH / 2.0 - PAR / 2.0); }

            p.pvx = (p.x - px) / dt;
            p.pvy = (p.y - py) / dt;
        }

        // ── Opponent paddle ───────────────────────────────────────────────────
        if self.is_single {
            let ai = &mut self.client_paddle;
            let (apx, apy) = (ai.x, ai.y);

            // Puck is "behind" the AI paddle when it has slipped between the paddle and the AI goal (y=0)
            let puck_behind    = self.puck.y < apy;
            let puck_in_half   = self.puck.y < TH / 2.0;
            let puck_approach  = self.puck.vy < -30.0;

            let (spd, tgt_x, tgt_y): (f32, f32, f32) = if puck_behind {
                // Chase the puck back — predict where it will be at interception
                let t = if self.puck.vy < -1.0 {
                    ((apy - self.puck.y) / (-self.puck.vy)).min(0.5)
                } else { 0.0 };
                let pred_x = (self.puck.x + self.puck.vx * t).clamp(PAR, TW - PAR);
                let ty = (self.puck.y - 20.0).max(PAR);
                (9.0, pred_x, ty)
            } else if puck_in_half || puck_approach {
                // Puck in AI half or approaching — intercept with prediction
                let t = if self.puck.vy.abs() > 10.0 {
                    ((apy - self.puck.y) / self.puck.vy.abs()).clamp(0.0, 0.4)
                } else { 0.0 };
                let pred_x = (self.puck.x + self.puck.vx * t).clamp(PAR, TW - PAR);
                let ty = (self.puck.y + 55.0).max(PAR).min(TH / 2.0 - PAR);
                (5.5, pred_x, ty)
            } else {
                // Puck in host half — return to defensive position
                (1.8, self.puck.x, 110.0)
            };

            ai.x += (tgt_x - ai.x) * (spd * dt).min(1.0);
            ai.y += (tgt_y - ai.y) * (spd * dt * 1.3).min(1.0);
            ai.x = ai.x.max(PAR).min(TW - PAR);
            ai.y = ai.y.max(PAR).min(TH/2.0 - PAR/2.0);
            ai.pvx = (ai.x - apx) / dt;
            ai.pvy = (ai.y - apy) / dt;
        } else {
            let op = if self.is_host { &mut self.client_paddle } else { &mut self.host_paddle };
            let (opx, opy) = (op.x, op.y);
            op.x += (self.target_opponent[0] - op.x) * 0.5;
            op.y += (self.target_opponent[1] - op.y) * 0.5;
            op.pvx = (op.x - opx) / dt;
            op.pvy = (op.y - opy) / dt;
        }

        // ── Puck ──────────────────────────────────────────────────────────────
        let auth_now = self.auth();

        if auth_now && self.countdown == 0.0 {
            // Just gained authority — snap to peer's last known position so our local puck.y
            // is on the correct side of the midline. Without this snap, the dead-reckoned
            // puck.y might be on the wrong side, the echo would flip target_puck wrong,
            // and auth() would immediately drop authority on the very next frame.
            if !self.prev_auth {
                self.puck.x  = self.target_puck.x;  self.puck.y  = self.target_puck.y;
                self.puck.vx = self.target_puck.vx; self.puck.vy = self.target_puck.vy;
            }
            self.puck.x += self.puck.vx * dt;
            self.puck.y += self.puck.vy * dt;

            // Friction
            let sp = (self.puck.vx*self.puck.vx + self.puck.vy*self.puck.vy).sqrt();
            if sp > 0.0 {
                let loss = (FRICTION * sp * dt).min(sp);
                self.puck.vx -= self.puck.vx/sp * loss;
                self.puck.vy -= self.puck.vy/sp * loss;
            }

            // Corner fillets
            let mut wh = false;
            let (px, py) = (self.puck.x, self.puck.y);
            wh |= collide_corner_puck(&mut self.puck, CR,    CR,    px<CR    && py<CR);
            wh |= collide_corner_puck(&mut self.puck, TW-CR, CR,    px>TW-CR && py<CR);
            wh |= collide_corner_puck(&mut self.puck, CR,    TH-CR, px<CR    && py>TH-CR);
            wh |= collide_corner_puck(&mut self.puck, TW-CR, TH-CR, px>TW-CR && py>TH-CR);

            // Side walls
            if      self.puck.x < PR    { self.puck.x = PR;    self.puck.vx =  self.puck.vx.abs()*WALL_REST; wh=true; }
            else if self.puck.x > TW-PR { self.puck.x = TW-PR; self.puck.vx = -self.puck.vx.abs()*WALL_REST; wh=true; }
            self.puck.x = self.puck.x.clamp(PR, TW - PR); // hard safety clamp

            // End walls
            let in_gap = (self.puck.x - TW/2.0).abs() < GOAL_W/2.0 + PR*0.6;
            if !in_gap {
                if      self.puck.y < PR    { self.puck.y = PR;    self.puck.vy =  self.puck.vy.abs()*WALL_REST; wh=true; }
                else if self.puck.y > TH-PR { self.puck.y = TH-PR; self.puck.vy = -self.puck.vy.abs()*WALL_REST; wh=true; }
            }

            // Goal posts
            wh |= collide_goal_post(&mut self.puck, GX,          0.0);
            wh |= collide_goal_post(&mut self.puck, GX+GOAL_W,   0.0);
            wh |= collide_goal_post(&mut self.puck, GX,           TH);
            wh |= collide_goal_post(&mut self.puck, GX+GOAL_W,    TH);
            if wh { self.wall_hit = 1; self.wall_flash = 1.0; }

            // Goals — score when puck centre crosses the goal line
            if self.puck.y < 0.0 {
                self.score[0] += 1; self.goal_scored = 1;
                self.goal_flash = 1.0; self.score_flash[0] = 1.0;
                self.reset_puck(); self.countdown = 2.5;
            } else if self.puck.y > TH {
                self.score[1] += 1; self.goal_scored = 1;
                self.goal_flash = 1.0; self.score_flash[1] = 1.0;
                self.reset_puck(); self.countdown = 2.5;
            }

            // Paddle collisions
            let hp = self.host_paddle.clone();
            let cp = self.client_paddle.clone();
            if collide_paddle_puck(&mut self.puck, &hp) { self.hit = 1; }
            if collide_paddle_puck(&mut self.puck, &cp) { self.hit = 1; }

            // Speed clamp (max only)
            let cs = (self.puck.vx*self.puck.vx + self.puck.vy*self.puck.vy).sqrt();
            if cs > MAX_SPEED { self.puck.vx=self.puck.vx/cs*MAX_SPEED; self.puck.vy=self.puck.vy/cs*MAX_SPEED; }

            // Echo authoritative puck into target_puck so auth() stays stable next frame
            // (prevents stale target_puck from causing false authority loss after a goal reset)
            self.target_puck = self.puck.clone();

        } else if auth_now {
            // Auth but in countdown: puck already at center from reset_puck(), hold it there.
            // Echo so auth() sees center next frame and doesn't incorrectly drop authority.
            self.target_puck = self.puck.clone();

        } else if self.countdown > 0.0 {
            // Non-auth during countdown: snap immediately to whatever auth sent (should be center).
            self.puck.x  = self.target_puck.x;  self.puck.y  = self.target_puck.y;
            self.puck.vx = self.target_puck.vx; self.puck.vy = self.target_puck.vy;

        } else {
            // Dead reckoning — blend toward peer's authoritative state
            self.puck.x += self.puck.vx * dt;
            self.puck.y += self.puck.vy * dt;
            let sp2 = (self.puck.vx*self.puck.vx + self.puck.vy*self.puck.vy).sqrt();
            if sp2 > 0.0 {
                let loss = (FRICTION * sp2 * dt).min(sp2);
                self.puck.vx -= self.puck.vx/sp2*loss; self.puck.vy -= self.puck.vy/sp2*loss;
            }
            if      self.puck.x < PR    { self.puck.x = PR;    self.puck.vx =  self.puck.vx.abs()*WALL_REST; }
            else if self.puck.x > TW-PR { self.puck.x = TW-PR; self.puck.vx = -self.puck.vx.abs()*WALL_REST; }
            let ig2 = (self.puck.x - TW/2.0).abs() < GOAL_W/2.0 + PR*0.6;
            if !ig2 {
                if      self.puck.y < PR    { self.puck.y = PR;    self.puck.vy =  self.puck.vy.abs()*WALL_REST; }
                else if self.puck.y > TH-PR { self.puck.y = TH-PR; self.puck.vy = -self.puck.vy.abs()*WALL_REST; }
            }
            // Dead-reckoned puck passed a goal — snap to center immediately;
            // auth side will send the score + reset shortly via net.
            if self.puck.y < 0.0 || self.puck.y > TH {
                self.reset_puck();
                self.target_puck = Puck { x:TW/2.0, y:TH/2.0, vx:0.0, vy:0.0 };
            }

            // Blend toward authoritative peer state
            let ex = self.target_puck.x - self.puck.x;
            let ey = self.target_puck.y - self.puck.y;
            let err = (ex*ex + ey*ey).sqrt();

            // If peer already reset puck to center (after goal), snap immediately
            let peer_reset = (self.target_puck.x - TW/2.0).abs() < 10.0
                && (self.target_puck.y - TH/2.0).abs() < 10.0
                && self.target_puck.vx.abs() < 1.0
                && self.target_puck.vy.abs() < 1.0;
            if peer_reset || err > 120.0 {
                self.puck.x=self.target_puck.x; self.puck.y=self.target_puck.y;
                self.puck.vx=self.target_puck.vx; self.puck.vy=self.target_puck.vy;
            } else if err > 1.0 {
                self.puck.x += ex*0.28; self.puck.y += ey*0.28;
                self.puck.vx += (self.target_puck.vx - self.puck.vx)*0.40;
                self.puck.vy += (self.target_puck.vy - self.puck.vy)*0.40;
            }
        }

        self.prev_auth = auth_now;

        // FX decay
        self.goal_flash     = (self.goal_flash     - dt*2.5).max(0.0);
        self.wall_flash     = (self.wall_flash     - dt*7.0).max(0.0);
        self.score_flash[0] = (self.score_flash[0] - dt*1.8).max(0.0);
        self.score_flash[1] = (self.score_flash[1] - dt*1.8).max(0.0);
    }

    fn to_render(&self) -> RenderState {
        let spd = (self.puck.vx*self.puck.vx + self.puck.vy*self.puck.vy).sqrt();
        RenderState {
            puck:          [self.puck.x,           self.puck.y],
            puck_speed:    spd,
            host_paddle:   [self.host_paddle.x,    self.host_paddle.y],
            client_paddle: [self.client_paddle.x,  self.client_paddle.y],
            score:         self.score,
            wall_flash:    self.wall_flash,
            goal_flash:    self.goal_flash,
            score_flash:   self.score_flash,
            hit:           self.hit,
            wall_hit:      self.wall_hit,
            goal_scored:   self.goal_scored,
            countdown:     self.countdown,
        }
    }

    fn net_msg(&self) -> Option<String> {
        if self.is_single { return None; }
        // Always send puck + isHostAuth every packet.
        // Receiver ignores puck data unless isHostAuth matches who should be sending.
        // This eliminates the linger/missed-handoff problem entirely.
        let is_host_auth = if self.is_host { self.prev_auth } else { !self.prev_auth };
        if self.is_host {
            Some(serde_json::json!({
                "type": "state",
                "hostPaddle": [self.host_paddle.x, self.host_paddle.y],
                "puck":       [self.puck.x, self.puck.y],
                "vel":        [self.puck.vx, self.puck.vy],
                "score":      self.score,
                "countdown":  self.countdown,
                "isHostAuth": is_host_auth,
            }).to_string())
        } else {
            Some(serde_json::json!({
                "type": "input",
                "pos":        [self.client_paddle.x, self.client_paddle.y],
                "puck":       [self.puck.x, self.puck.y],
                "vel":        [self.puck.vx, self.puck.vy],
                "score":      self.score,
                "countdown":  self.countdown,
                "isHostAuth": is_host_auth,
            }).to_string())
        }
    }

    fn apply_net(&mut self, msg: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(msg) else { return };
        // isHostAuth: true = host is currently authoritative
        let is_host_auth = v["isHostAuth"].as_bool().unwrap_or(true);
        // Detect authority transitions — on a handoff, always accept puck data even
        // if the sender just dropped authority. This ensures packet loss of the last
        // auth packet doesn't cause a permanent deadlock: the first non-auth packet
        // from the old owner still delivers the crossing position.
        let auth_changed = is_host_auth != self.prev_recv_host_auth;
        self.prev_recv_host_auth = is_host_auth;

        // Parse received score for use in both branches (needed for stale-packet guard)
        let recv_score = v["score"].as_array().map(|s| [
            s[0].as_u64().unwrap_or(0) as u32,
            s[1].as_u64().unwrap_or(0) as u32,
        ]);
        // A packet is from a past round if its score sum is less than ours — any countdown
        // value in it predates the current goal and must be ignored to prevent a post-goal
        // countdown=2.5 from being overwritten by a stale countdown=0.
        let recv_sum   = recv_score.map_or(0, |s| s[0] + s[1]);
        let local_sum  = self.score[0] + self.score[1];
        let fresh_round = recv_sum >= local_sum;

        if self.is_host && v["type"] == "input" {
            if let Some(pos) = v["pos"].as_array() {
                self.target_opponent = [pos[0].as_f64().unwrap_or(0.0) as f32,
                                        pos[1].as_f64().unwrap_or(0.0) as f32];
            }
            // Score always applies via max (safe — can only increase)
            if let Some(s) = recv_score {
                self.score[0] = self.score[0].max(s[0]);
                self.score[1] = self.score[1].max(s[1]);
            }
            // Accept puck+countdown from client when: client is auth (!is_host_auth) OR authority just changed
            if !is_host_auth || auth_changed {
                if let (Some(p), Some(vel)) = (v["puck"].as_array(), v["vel"].as_array()) {
                    self.target_puck = Puck {
                        x:  p[0].as_f64().unwrap_or(0.0) as f32,
                        y:  p[1].as_f64().unwrap_or(0.0) as f32,
                        vx: vel[0].as_f64().unwrap_or(0.0) as f32,
                        vy: vel[1].as_f64().unwrap_or(0.0) as f32,
                    };
                }
                // Sync countdown from auth freely, but reject stale pre-goal packets
                // (those have recv_sum < local_sum and would carry countdown=0 after a goal).
                if fresh_round {
                    if let Some(c) = v["countdown"].as_f64() { self.countdown = c as f32; }
                }
            }
        } else if !self.is_host && v["type"] == "state" {
            if let Some(hp) = v["hostPaddle"].as_array() {
                self.target_opponent = [hp[0].as_f64().unwrap_or(0.0) as f32,
                                        hp[1].as_f64().unwrap_or(0.0) as f32];
            }
            // Score always applies via max
            if let Some(s) = recv_score {
                self.score[0] = self.score[0].max(s[0]);
                self.score[1] = self.score[1].max(s[1]);
            }
            // Accept puck+countdown from host when: host is auth (is_host_auth) OR authority just changed
            if is_host_auth || auth_changed {
                if let (Some(p), Some(vel)) = (v["puck"].as_array(), v["vel"].as_array()) {
                    self.target_puck = Puck {
                        x:  p[0].as_f64().unwrap_or(0.0) as f32,
                        y:  p[1].as_f64().unwrap_or(0.0) as f32,
                        vx: vel[0].as_f64().unwrap_or(0.0) as f32,
                        vy: vel[1].as_f64().unwrap_or(0.0) as f32,
                    };
                }
                // Sync countdown from auth freely, but reject stale pre-goal packets
                if fresh_round {
                    if let Some(c) = v["countdown"].as_f64() { self.countdown = c as f32; }
                }
            }
        }
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_game(
    engine:           State<'_, GameEngine>,
    transport:        State<'_, TransportState>,
    is_host:          bool,
    is_single_player: bool,
    channel:          Channel<RenderState>,
) -> Result<(), String> {
    if engine.running.swap(true, Ordering::SeqCst) {
        return Ok(()); // already running
    }

    // Reset pointer to starting position
    *engine.pointer.lock().unwrap() = if is_host { [TW/2.0, TH-120.0] } else { [TW/2.0, 120.0] };

    // Cloneable handles passed into the async task
    let running  = engine.running.clone();
    let paused   = engine.paused.clone();
    let pointer  = engine.pointer.clone();

    // iroh connection handle (clone the Arc, not the option)
    let connection = transport.connection.clone();

    // Subscribe to the broadcast channel fed by the recv loop in transport.rs
    let mut net_rx = transport.msg_tx.subscribe();

    tokio::spawn(async move {
        let mut gs       = GameState::new(is_host, is_single_player);
        let mut interval = tokio::time::interval(Duration::from_nanos(16_666_667));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last     = Instant::now();

        while running.load(Ordering::Relaxed) {
            interval.tick().await;
            if paused.load(Ordering::Relaxed) { continue; }
            let now = Instant::now();
            let dt  = now.duration_since(last).as_secs_f32().min(0.05);
            last = now;

            // Drain ALL pending network messages (take only the latest if multiple arrived)
            loop {
                match net_rx.try_recv() {
                    Ok(msg)                                => { gs.apply_net(&msg); }
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty)   => break,
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => break, // stale, skip
                    Err(_) => break,
                }
            }

            // Physics tick
            let ptr = *pointer.lock().unwrap();
            gs.update(dt, ptr);

            // Send every frame — ~14KB/s total, trivial for LAN and QUIC.
            // Throttling to 30Hz was causing authority deadlocks: the echo could
            // cross the midline on a non-send frame, dropping authority without
            // ever notifying the peer, leaving both sides non-authoritative.
            if let Some(msg) = gs.net_msg() {
                if let Some(ref conn) = *connection.lock().await {
                    let _ = conn.send_datagram(bytes::Bytes::from(msg.into_bytes()));
                }
            }

            // Push render state to JS — this is the only remaining IPC
            if channel.send(gs.to_render()).is_err() {
                break; // JS closed the channel (navigated away)
            }
        }

        running.store(false, Ordering::Relaxed);
    });

    Ok(())
}

#[tauri::command]
pub fn stop_game(engine: State<'_, GameEngine>) {
    engine.paused.store(false, Ordering::Relaxed);
    engine.running.store(false, Ordering::Relaxed);
}

#[tauri::command]
pub fn pause_game(engine: State<'_, GameEngine>) {
    engine.paused.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub fn resume_game(engine: State<'_, GameEngine>) {
    engine.paused.store(false, Ordering::Relaxed);
}

#[tauri::command]
pub fn set_pointer(engine: State<'_, GameEngine>, x: f32, y: f32) {
    *engine.pointer.lock().unwrap() = [x, y];
}
