use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use crate::transport::WebRtcTransportState;
use crate::config::*;
use crate::config::ai;
use crate::config::interpolation;
use crate::physics::{Puck, Paddle, collide_paddle_puck, collide_corner_puck, collide_goal_post};
use matchbox_socket::Packet;
use std::net::SocketAddr;

// Convenience aliases using config constants
const TW: f32 = TABLE_WIDTH;
const TH: f32 = TABLE_HEIGHT;
const PR: f32 = PUCK_RADIUS;
const PAR: f32 = PADDLE_RADIUS;
const GOAL_W: f32 = GOAL_WIDTH;
const GX: f32 = (TW - GOAL_W) / 2.0;
const CR: f32 = CORNER_RADIUS;

// ── Public state managed by Tauri ─────────────────────────────────────────────
pub struct GameEngine {
    pub running: Arc<AtomicBool>,
    pub paused:  Arc<AtomicBool>,
    pub pointer: Arc<Mutex<[f32; 2]>>,
    /// Handle to the active game loop task so it can be forcibly aborted.
    /// Uses std::sync::Mutex so stop_game can stay a sync command.
    pub task:    Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl GameEngine {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            paused:  Arc::new(AtomicBool::new(false)),
            pointer: Arc::new(Mutex::new([TW / 2.0, TH - 120.0])),
            task:    Mutex::new(None),
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
    prev_recv_host_auth: bool,
    is_host:             bool,
    is_single:           bool,

    // authority handoff smoothing
    handoff_blend:       f32,
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
            prev_recv_host_auth: true,
            is_host, is_single,
            handoff_blend: 0.0,
        }
    }

    fn host_is_authoritative(&self) -> bool {
        if self.is_single { return true; }

        let mid = TH / 2.0;
        let prev_host_auth = if self.is_host { self.prev_auth } else { !self.prev_auth };

        if self.puck.y > mid + AUTH_HYSTERESIS {
            true
        } else if self.puck.y < mid - AUTH_HYSTERESIS {
            false
        } else {
            prev_host_auth
        }
    }

    fn auth(&self) -> bool {
        if self.is_single { return true; }

        // Multiplayer uses split authority: the side containing the puck owns
        // puck simulation and scoring. Inside the center handoff band we keep
        // the previous owner, which makes the tie-break deterministic on both
        // peers and prevents rapid authority flapping near midfield.
        self.host_is_authoritative() == self.is_host
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
            let puck_coming_fast = self.puck.vy < -80.0;

            let (spd, tgt_x, tgt_y): (f32, f32, f32) = if puck_behind {
                // Puck slipped past AI paddle — chase directly (but slower, so player can score)
                let ty = (self.puck.y - 15.0).max(PAR);
                (ai::CHASE_SPEED, self.puck.x, ty)
            } else if puck_in_half && puck_approach {
                // Puck in AI half and approaching — intercept with prediction
                let t = if self.puck.vy < -10.0 {
                    ((apy - self.puck.y) / self.puck.vy.abs()).clamp(0.0, ai::PREDICTION_TIME)
                } else { 0.0 };
                let pred_x = (self.puck.x + self.puck.vx * t).clamp(PAR, TW - PAR);
                
                // If puck coming fast, be more defensive; otherwise intercept aggressively
                let ty = if puck_coming_fast {
                    // Defensive: stay closer to goal, react to puck X
                    (self.puck.y - 60.0).clamp(ai::DEFENSIVE_Y, TH / 2.0 - PAR / 2.0)
                } else {
                    // Aggressive: move up to intercept
                    (self.puck.y - ai::BLOCK_DISTANCE).clamp(PAR, TH / 2.0 - PAR / 2.0)
                };
                (ai::INTERCEPT_SPEED, pred_x, ty)
            } else {
                // Puck in host half or not approaching — return to centered defensive home
                (ai::RETURN_SPEED, TW / 2.0, ai::HOME_Y)
            };

            // Smooth lerp toward target with AI reaction simulation
            let lerp = ai::REACTION_LERP;
            ai.x = ai.x + (tgt_x - ai.x) * lerp;
            ai.y = ai.y + (tgt_y - ai.y) * lerp;
            
            // Clamp to table bounds
            ai.x = ai.x.max(PAR).min(TW - PAR);
            ai.y = ai.y.max(PAR).min(TH/2.0 - PAR/2.0);
            
            // Velocity for physics collisions
            ai.pvx = (ai.x - apx) / dt;
            ai.pvy = (ai.y - apy) / dt;
        } else {
            let op = if self.is_host { &mut self.client_paddle } else { &mut self.host_paddle };
            let (opx, opy) = (op.x, op.y);
            // Use config constant for opponent paddle interpolation
            op.x += (self.target_opponent[0] - op.x) * interpolation::OPPONENT_PADDLE_LERP;
            op.y += (self.target_opponent[1] - op.y) * interpolation::OPPONENT_PADDLE_LERP;
            op.pvx = (op.x - opx) / dt;
            op.pvy = (op.y - opy) / dt;
        }

        // ── Puck ──────────────────────────────────────────────────────────────
        let auth_now = self.auth();

        if auth_now && self.countdown == 0.0 {
            // Just gained authority — blend to peer's last known puck state smoothly
            // over ~3 frames to avoid visible "teleport" at midfield
            if !self.prev_auth {
                // Start handoff blend
                self.handoff_blend = 0.0;
            }
            
            // Smooth handoff: blend over multiple frames
            if self.handoff_blend < 1.0 {
                self.handoff_blend += interpolation::HANDOFF_BLEND;
                let blend = self.handoff_blend.min(1.0);
                self.puck.x  = self.puck.x + (self.target_puck.x - self.puck.x) * blend;
                self.puck.y  = self.puck.y + (self.target_puck.y - self.puck.y) * blend;
                self.puck.vx = self.puck.vx + (self.target_puck.vx - self.puck.vx) * blend;
                self.puck.vy = self.puck.vy + (self.target_puck.vy - self.puck.vy) * blend;
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

            // Echo authoritative puck into target_puck so the next ownership
            // decision uses the freshest crossing/reset state.
            self.target_puck = self.puck.clone();

        } else if auth_now {
            // Auth but in countdown: puck already sits at center from reset_puck().
            // Echo it so the sticky handoff band keeps both peers aligned.
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
                self.countdown = 2.5; // prevent physics running before auth side confirms reset
                self.target_puck = Puck { x:TW/2.0, y:TH/2.0, vx:0.0, vy:0.0 };
            }

            // Blend toward authoritative peer state with adaptive dead reckoning
            let ex = self.target_puck.x - self.puck.x;
            let ey = self.target_puck.y - self.puck.y;
            let err = (ex*ex + ey*ey).sqrt();

            // Adaptive blend factor: smooth when close, snappy when diverged
            let blend = (err / interpolation::ADAPTIVE_ERROR_THRESHOLD)
                .clamp(interpolation::MIN_BLEND, interpolation::MAX_BLEND);

            // If peer already reset puck to center (after goal), snap immediately
            let peer_reset = (self.target_puck.x - TW/2.0).abs() < 10.0
                && (self.target_puck.y - TH/2.0).abs() < 10.0
                && self.target_puck.vx.abs() < 1.0
                && self.target_puck.vy.abs() < 1.0;
            if peer_reset || err > interpolation::DEAD_RECKONING_SNAP_THRESHOLD {
                self.puck.x=self.target_puck.x; self.puck.y=self.target_puck.y;
                self.puck.vx=self.target_puck.vx; self.puck.vy=self.target_puck.vy;
            } else if err > 1.0 {
                // Use adaptive blend for position, fixed blend for velocity
                self.puck.x += ex * blend;
                self.puck.y += ey * blend;
                self.puck.vx += (self.target_puck.vx - self.puck.vx) * interpolation::PUCK_VELOCITY_LERP;
                self.puck.vy += (self.target_puck.vy - self.puck.vy) * interpolation::PUCK_VELOCITY_LERP;
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
        // Also send audio event flags so peer can play sounds.
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
                // Audio event flags
                "hit":        self.hit,
                "wall_hit":   self.wall_hit,
                "goal_scored": self.goal_scored,
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
                // Audio event flags
                "hit":        self.hit,
                "wall_hit":   self.wall_hit,
                "goal_scored": self.goal_scored,
            }).to_string())
        }
    }

    fn apply_net(&mut self, msg: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(msg) else {
            log::error!("Failed to parse network message: {}", msg);
            return;
        };
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

        // Parse audio event flags from peer
        let recv_hit = v["hit"].as_u64().map(|n| n as u8).unwrap_or(0);
        let recv_wall_hit = v["wall_hit"].as_u64().map(|n| n as u8).unwrap_or(0);
        let recv_goal_scored = v["goal_scored"].as_u64().map(|n| n as u8).unwrap_or(0);

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
            // Accept puck+countdown from client when: client is auth (!is_host_auth)
            // OR authority just changed. Ignore stale-round puck packets so an
            // old pre-goal state cannot pull us away from center after scoring.
            if !is_host_auth || auth_changed {
                if fresh_round {
                    if let (Some(p), Some(vel)) = (v["puck"].as_array(), v["vel"].as_array()) {
                        self.target_puck = Puck {
                            x:  p[0].as_f64().unwrap_or(0.0) as f32,
                            y:  p[1].as_f64().unwrap_or(0.0) as f32,
                            vx: vel[0].as_f64().unwrap_or(0.0) as f32,
                            vy: vel[1].as_f64().unwrap_or(0.0) as f32,
                        };
                    }
                }
                // Sync countdown from auth freely, but reject stale pre-goal packets
                // (those have recv_sum < local_sum and would carry countdown=0 after a goal).
                if fresh_round {
                    if let Some(c) = v["countdown"].as_f64() { self.countdown = c as f32; }
                }
            }
            // Apply audio events from peer (always accept — they're one-frame events)
            if recv_hit != 0 { self.hit = 1; }
            if recv_wall_hit != 0 { self.wall_hit = 1; }
            if recv_goal_scored != 0 { self.goal_scored = 1; }
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
            // Accept puck+countdown from host when: host is auth (is_host_auth)
            // OR authority just changed. Ignore stale-round puck packets so an
            // old pre-goal state cannot pull us away from center after scoring.
            if is_host_auth || auth_changed {
                if fresh_round {
                    if let (Some(p), Some(vel)) = (v["puck"].as_array(), v["vel"].as_array()) {
                        self.target_puck = Puck {
                            x:  p[0].as_f64().unwrap_or(0.0) as f32,
                            y:  p[1].as_f64().unwrap_or(0.0) as f32,
                            vx: vel[0].as_f64().unwrap_or(0.0) as f32,
                            vy: vel[1].as_f64().unwrap_or(0.0) as f32,
                        };
                    }
                }
                // Sync countdown from auth freely, but reject stale pre-goal packets
                if fresh_round {
                    if let Some(c) = v["countdown"].as_f64() { self.countdown = c as f32; }
                }
            }
            // Apply audio events from peer (always accept — they're one-frame events)
            if recv_hit != 0 { self.hit = 1; }
            if recv_wall_hit != 0 { self.wall_hit = 1; }
            if recv_goal_scored != 0 { self.goal_scored = 1; }
        }
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_game(
    engine:           State<'_, GameEngine>,
    transport:        State<'_, WebRtcTransportState>,
    udp:              State<'_, crate::udp_transport::UdpState>,
    is_host:          bool,
    is_single_player: bool,
    channel:          Channel<RenderState>,
    use_udp:          bool,
) -> Result<(), String> {
    // Abort any existing game task before starting a new one.
    // abort() is sufficient — an aborted task is cancelled at its next .await
    // point and never reaches the final running.store(false), eliminating the
    // race condition where the old task's cleanup killed the new game.
    {
        let old = engine.task.lock().unwrap().take();
        if let Some(h) = old { h.abort(); }
    }

    engine.running.store(false, Ordering::SeqCst);
    engine.paused.store(false, Ordering::SeqCst);
    *engine.pointer.lock().unwrap() = if is_host { [TW/2.0, TH-120.0] } else { [TW/2.0, 120.0] };
    engine.running.store(true, Ordering::SeqCst);

    // Cloneable handles passed into the async task
    let running  = engine.running.clone();
    let paused   = engine.paused.clone();
    let pointer  = engine.pointer.clone();

    // WebRTC socket and peer ID (use getter methods)
    let webrtc_socket = transport.get_socket();
    let webrtc_peer_id = transport.get_peer_id();

    // clone pieces of the UDP state so we can move them into the async task
    let udp_socket = udp.socket.clone();
    let udp_peer = udp.peer.clone();

    // choose which broadcast channel to subscribe to
    let mut net_rx = if use_udp {
        udp.msg_tx.subscribe()
    } else {
        transport.msg_tx.subscribe()
    };

    let handle = tokio::spawn(async move {
        let mut gs       = GameState::new(is_host, is_single_player);
        let mut interval = tokio::time::interval(Duration::from_nanos(16_666_667));
        // Skip missed ticks to prevent spiral of death — keeps game smooth even if scheduler delays
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last     = Instant::now();

        while running.load(Ordering::Relaxed) {
            interval.tick().await;
            if paused.load(Ordering::Relaxed) { continue; }
            let now = Instant::now();
            // Cap at 33 ms (2 frames) — prevents large physics jumps after scheduler delays
            let dt  = now.duration_since(last).as_secs_f32().min(0.033);
            last = now;

            // Drain ALL pending network messages (take only the latest if multiple arrived)
            loop {
                match net_rx.try_recv() {
                    Ok(msg)                                => { gs.apply_net(&msg); }
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty)   => break,
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => break,
                    Err(_) => break,
                }
            }

            // Physics tick
            let ptr = *pointer.lock().unwrap();
            gs.update(dt, ptr);

            if let Some(msg) = gs.net_msg() {
                if use_udp {
                    let peer_opt: Option<SocketAddr> = { udp_peer.lock().await.clone() };
                    if let Some(peer) = peer_opt {
                        if let Some(sock) = &*udp_socket.lock().await {
                            let _ = sock.send_to(msg.as_bytes(), peer).await;
                        }
                    }
                } else {
                    let mut socket_guard = webrtc_socket.lock().await;
                    let peer_guard = webrtc_peer_id.lock().await;
                    if let (Some(socket), Some(peer)) = (socket_guard.as_mut(), peer_guard.as_ref()) {
                        if let Ok(channel) = socket.get_channel_mut(0) {
                            let packet = Packet::from(msg.into_bytes());
                            let _ = channel.send(packet, *peer);
                        }
                    }
                }
            }

            // Push render state to JS
            if channel.send(gs.to_render()).is_err() {
                break; // JS closed the channel (navigated away)
            }
        }

        running.store(false, Ordering::Relaxed);
    });

    *engine.task.lock().unwrap() = Some(handle);

    Ok(())
}

#[tauri::command]
pub fn stop_game(engine: State<'_, GameEngine>) {
    if let Some(h) = engine.task.lock().unwrap().take() {
        h.abort();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_gamestate(is_host: bool, is_single: bool) -> GameState {
        GameState::new(is_host, is_single)
    }

    #[test]
    fn test_host_net_msg_format() {
        let mut gs = create_test_gamestate(true, false);
        let msg = gs.net_msg().expect("Should produce a message");
        
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["type"], "state");
        assert!(v["hostPaddle"].is_array());
        assert!(v["puck"].is_array());
        assert!(v["vel"].is_array());
        assert!(v["score"].is_array());
        assert!(v["isHostAuth"].is_boolean());
    }

    #[test]
    fn test_client_net_msg_format() {
        let mut gs = create_test_gamestate(false, false);
        let msg = gs.net_msg().expect("Should produce a message");
        
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["type"], "input");
        assert!(v["pos"].is_array());
        assert!(v["puck"].is_array());
        assert!(v["vel"].is_array());
        assert!(v["score"].is_array());
        assert!(v["isHostAuth"].is_boolean());
    }

    #[test]
    fn test_single_player_no_net_msg() {
        let gs = create_test_gamestate(true, true);
        assert!(gs.net_msg().is_none());
    }

    #[test]
    fn test_apply_net_parses_host_state() {
        let mut gs = create_test_gamestate(false, false); // client
        let msg = json!({
            "type": "state",
            "hostPaddle": [100.0, 200.0],
            "puck": [50.0, 60.0],
            "vel": [10.0, -5.0],
            "score": [1, 0],
            "countdown": 2.0,
            "isHostAuth": true
        }).to_string();
        
        gs.apply_net(&msg);
        
        assert_eq!(gs.target_opponent[0], 100.0);
        assert_eq!(gs.target_opponent[1], 200.0);
        assert_eq!(gs.score[0], 1);
        assert_eq!(gs.countdown, 2.0);
    }

    #[test]
    fn test_apply_net_parses_client_input() {
        let mut gs = create_test_gamestate(true, false); // host
        let msg = json!({
            "type": "input",
            "pos": [150.0, 100.0],
            "puck": [50.0, 60.0],
            "vel": [10.0, -5.0],
            "score": [0, 1],
            "countdown": 1.5,
            "isHostAuth": true
        }).to_string();
        
        gs.apply_net(&msg);
        
        assert_eq!(gs.target_opponent[0], 150.0);
        assert_eq!(gs.target_opponent[1], 100.0);
        assert_eq!(gs.score[1], 1);
        assert_eq!(gs.countdown, 1.5);
    }

    #[test]
    fn test_apply_net_rejects_malformed_json() {
        let mut gs = create_test_gamestate(true, false);
        let initial_score = gs.score;
        
        gs.apply_net("not valid json");
        
        // Score should be unchanged (silent failure with log)
        assert_eq!(gs.score, initial_score);
    }

    #[test]
    fn test_apply_net_score_max_applies() {
        let mut gs = create_test_gamestate(true, false);
        gs.score = [2, 1]; // Local score
        
        // Remote has higher score for client
        let msg = json!({
            "type": "input",
            "pos": [100.0, 100.0],
            "score": [1, 3],
            "isHostAuth": true
        }).to_string();
        
        gs.apply_net(&msg);
        
        // Should take max: [2, 3]
        assert_eq!(gs.score[0], 2);
        assert_eq!(gs.score[1], 3);
    }

    #[test]
    fn test_stale_packet_guard() {
        let mut gs = create_test_gamestate(false, false);
        gs.score = [1, 0]; // Local already scored
        
        // Stale packet from before goal (score sum 0 < 1)
        let stale_msg = json!({
            "type": "state",
            "hostPaddle": [100.0, 200.0],
            "puck": [50.0, 60.0],
            "vel": [10.0, -5.0],
            "score": [0, 0],
            "countdown": 0.0,
            "isHostAuth": false
        }).to_string();
        
        gs.countdown = 2.5; // Post-goal countdown
        gs.apply_net(&stale_msg);
        
        // Countdown should NOT be overwritten by stale packet
        assert_eq!(gs.countdown, 2.5);
    }

    #[test]
    fn test_authority_detection() {
        let mut gs_host = create_test_gamestate(true, false);
        let mut gs_client = create_test_gamestate(false, false);
        
        // Puck in host half - host should be authoritative
        gs_host.puck.y = TH - 100.0;
        gs_client.puck.y = TH - 100.0;
        
        assert!(gs_host.auth());
        assert!(!gs_client.auth());
        
        // Puck in client half - client should be authoritative
        gs_host.puck.y = 100.0;
        gs_client.puck.y = 100.0;
        
        assert!(!gs_host.auth());
        assert!(gs_client.auth());
    }

    #[test]
    fn test_single_player_always_authoritative() {
        let mut gs = create_test_gamestate(true, true);
        gs.puck.y = 100.0;
        assert!(gs.auth());
        
        gs.puck.y = TH - 100.0;
        assert!(gs.auth());
    }
}
