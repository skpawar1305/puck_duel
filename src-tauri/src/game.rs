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
    pub version_mismatch: bool,
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

    // single-player AI state
    ai_think_timer:      f32,
    ai_target:           [f32; 2],

    version_mismatch:    bool,
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
            ai_think_timer: 0.0,
            ai_target: [TW / 2.0, ai::HOME_Y],
            version_mismatch: false,
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

    fn reset_puck(&mut self, loser: Option<usize>) {
        let y = match loser {
            Some(0) => TH * 0.75,
            Some(1) => TH * 0.25,
            _ => TH / 2.0,
        };
        self.puck = Puck { x: TW / 2.0, y, vx: 0.0, vy: 0.0 };
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

            self.ai_think_timer -= dt;
            if self.ai_think_timer <= 0.0 {
                self.ai_think_timer = ai::THINK_INTERVAL;

                let (spd, base_x, base_y): (f32, f32, f32) = if puck_behind {
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
                        (self.puck.y - 60.0).clamp(ai::DEFENSIVE_Y, TH / 2.0 - PAR / 2.0)
                    } else {
                        (self.puck.y - ai::BLOCK_DISTANCE).clamp(PAR, TH / 2.0 - PAR / 2.0)
                    };
                    (ai::INTERCEPT_SPEED, pred_x, ty)
                } else if puck_in_half {
                    // Puck is in AI half but not approaching (e.g. stopped after spawn, or moving slow/sideways)
                    if apy < self.puck.y - 12.0 {
                        // AI is behind it, strike it downward!
                        (ai::INTERCEPT_SPEED, self.puck.x, self.puck.y + 20.0)
                    } else {
                        // AI is dangerously low or alongside, move up behind the puck to avoid own-goals
                        (ai::INTERCEPT_SPEED, self.puck.x, (self.puck.y - 35.0).max(PAR))
                    }
                } else {
                    // Puck in host half or not approaching — return to centered defensive home
                    (ai::RETURN_SPEED, TW / 2.0, ai::HOME_Y)
                };

                // Small deterministic aim error to avoid robotic perfection.
                let noise_phase =
                    self.puck.x * 0.093 + self.puck.y * 0.071 + self.score[0] as f32 * 0.61 + self.score[1] as f32 * 0.83;
                let err_x = noise_phase.sin() * ai::AIM_ERROR_X;
                let err_y = (noise_phase * 1.7).cos() * ai::AIM_ERROR_Y;

                self.ai_target = [
                    (base_x + err_x).clamp(PAR, TW - PAR),
                    (base_y + err_y).clamp(PAR, TH / 2.0 - PAR / 2.0),
                ];

                // Store speed in target drift by shrinking think interval under pressure.
                if spd >= ai::INTERCEPT_SPEED && puck_coming_fast {
                    self.ai_think_timer *= 0.75;
                }
            }

            // Smooth movement toward last computed target, capped by speed for human-like inertia.
            let lerp = ai::REACTION_LERP;
            let desired_x = ai.x + (self.ai_target[0] - ai.x) * lerp;
            let desired_y = ai.y + (self.ai_target[1] - ai.y) * lerp;

            // Convert configured px/frame-like speeds to per-tick max step at 60Hz.
            let speed = if puck_behind {
                ai::CHASE_SPEED
            } else if puck_in_half && puck_approach {
                ai::INTERCEPT_SPEED
            } else {
                ai::RETURN_SPEED
            };
            let max_step = (speed * 60.0 * dt).max(0.001);

            let dx = desired_x - ai.x;
            let dy = desired_y - ai.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > max_step {
                ai.x += dx / dist * max_step;
                ai.y += dy / dist * max_step;
            } else {
                ai.x = desired_x;
                ai.y = desired_y;
            }
            
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

                // Preserve momentum on midline ownership gain when our local state
                // has nearly stopped but peer still reports meaningful velocity.
                let near_midline = (self.puck.y - TH / 2.0).abs() <= AUTH_HYSTERESIS * 2.0;
                let local_spd = (self.puck.vx * self.puck.vx + self.puck.vy * self.puck.vy).sqrt();
                let peer_spd = (self.target_puck.vx * self.target_puck.vx + self.target_puck.vy * self.target_puck.vy).sqrt();
                if near_midline && local_spd < 28.0 && peer_spd > 120.0 {
                    self.puck.vx = self.target_puck.vx * 0.92;
                    self.puck.vy = self.target_puck.vy * 0.92;
                }
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
            
            let substeps = 4;
            let sub_dt = dt / substeps as f32;
            let mut sub_hit = false;
            let mut sub_wall_hit = false;

            for _ in 0..substeps {
                self.puck.x += self.puck.vx * sub_dt;
                self.puck.y += self.puck.vy * sub_dt;

                // Avoid double damping while handoff interpolation is still active.
                if self.handoff_blend >= 1.0 {
                    let sp = (self.puck.vx*self.puck.vx + self.puck.vy*self.puck.vy).sqrt();
                    if sp > 0.0 {
                        let loss = (FRICTION * sp * sub_dt).min(sp);
                        self.puck.vx -= self.puck.vx/sp * loss;
                        self.puck.vy -= self.puck.vy/sp * loss;
                    }
                }

                // Corner fillets
                let (px, py) = (self.puck.x, self.puck.y);
                sub_wall_hit |= collide_corner_puck(&mut self.puck, CR,    CR,    px<CR    && py<CR);
                sub_wall_hit |= collide_corner_puck(&mut self.puck, TW-CR, CR,    px>TW-CR && py<CR);
                sub_wall_hit |= collide_corner_puck(&mut self.puck, CR,    TH-CR, px<CR    && py>TH-CR);
                sub_wall_hit |= collide_corner_puck(&mut self.puck, TW-CR, TH-CR, px>TW-CR && py>TH-CR);

                // Side walls
                if      self.puck.x < PR    { self.puck.x = PR;    self.puck.vx =  self.puck.vx.abs()*WALL_REST; sub_wall_hit=true; }
                else if self.puck.x > TW-PR { self.puck.x = TW-PR; self.puck.vx = -self.puck.vx.abs()*WALL_REST; sub_wall_hit=true; }
                self.puck.x = self.puck.x.clamp(PR, TW - PR); // hard safety clamp

                // End walls
                let in_gap = (self.puck.x - TW/2.0).abs() < GOAL_W/2.0 + PR*0.6;
                if !in_gap {
                    if      self.puck.y < PR    { self.puck.y = PR;    self.puck.vy =  self.puck.vy.abs()*WALL_REST; sub_wall_hit=true; }
                    else if self.puck.y > TH-PR { self.puck.y = TH-PR; self.puck.vy = -self.puck.vy.abs()*WALL_REST; sub_wall_hit=true; }
                }

                // Goal posts
                sub_wall_hit |= collide_goal_post(&mut self.puck, GX,          0.0);
                sub_wall_hit |= collide_goal_post(&mut self.puck, GX+GOAL_W,   0.0);
                sub_wall_hit |= collide_goal_post(&mut self.puck, GX,           TH);
                sub_wall_hit |= collide_goal_post(&mut self.puck, GX+GOAL_W,    TH);

                // Paddle collisions
                let hp = self.host_paddle.clone();
                let cp = self.client_paddle.clone();
                if collide_paddle_puck(&mut self.puck, &hp) { sub_hit = true; }
                if collide_paddle_puck(&mut self.puck, &cp) { sub_hit = true; }

                // Speed clamp (max only)
                let cs = (self.puck.vx*self.puck.vx + self.puck.vy*self.puck.vy).sqrt();
                if cs > MAX_SPEED { self.puck.vx=self.puck.vx/cs*MAX_SPEED; self.puck.vy=self.puck.vy/cs*MAX_SPEED; }
            }

            if sub_wall_hit { self.wall_hit = 1; self.wall_flash = 1.0; }
            if sub_hit { self.hit = 1; }

            // Goals — score when puck centre crosses the goal line
            if self.puck.y < 0.0 {
                self.score[0] += 1; self.goal_scored = 1;
                self.goal_flash = 1.0; self.score_flash[0] = 1.0;
                self.reset_puck(Some(1)); self.countdown = 2.5;
            } else if self.puck.y > TH {
                self.score[1] += 1; self.goal_scored = 1;
                self.goal_flash = 1.0; self.score_flash[1] = 1.0;
                self.reset_puck(Some(0)); self.countdown = 2.5;
            }

            // Echo authoritative puck into target_puck so the next ownership
            // decision uses the freshest crossing/reset state.
            self.target_puck = self.puck.clone();

        } else if auth_now {
            // Auth but in countdown: puck already sits at center from reset_puck().
            // Echo it so the sticky handoff band keeps both peers aligned.
            self.target_puck = self.puck.clone();
            self.handoff_blend = 0.0;

        } else if self.countdown > 0.0 {
            // Non-auth during countdown: snap immediately to whatever auth sent (should be center).
            self.puck.x  = self.target_puck.x;  self.puck.y  = self.target_puck.y;
            self.puck.vx = self.target_puck.vx; self.puck.vy = self.target_puck.vy;
            self.handoff_blend = 0.0;

        } else {
            self.handoff_blend = 0.0;
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
            // Dead-reckoned puck passed a goal — snap back immediately;
            // auth side will send the score + reset shortly via net.
            if self.puck.y < 0.0 || self.puck.y > TH {
                let loser = if self.puck.y < 0.0 { 1 } else { 0 };
                self.reset_puck(Some(loser));
                self.countdown = 2.5; // prevent physics running before auth side confirms reset
                self.target_puck = self.puck.clone();
            }

            // Blend toward authoritative peer state with adaptive dead reckoning
            let ex = self.target_puck.x - self.puck.x;
            let ey = self.target_puck.y - self.puck.y;
            let err = (ex*ex + ey*ey).sqrt();

            // Adaptive blend factor: smooth when close, snappy when diverged
            let blend = (err / interpolation::ADAPTIVE_ERROR_THRESHOLD)
                .clamp(interpolation::MIN_BLEND, interpolation::MAX_BLEND);

            // If peer already reset puck (after goal), snap immediately
            let target_y = self.target_puck.y;
            let center_or_side = (target_y - TH/2.0).abs() < 10.0 || (target_y - TH*0.25).abs() < 10.0 || (target_y - TH*0.75).abs() < 10.0;
            let peer_reset = (self.target_puck.x - TW/2.0).abs() < 10.0
                && center_or_side
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
            version_mismatch: self.version_mismatch,
        }
    }

    fn net_msg(&self) -> Option<Vec<u8>> {
        if self.is_single { return None; }
        let mut buf = Vec::with_capacity(33);
        let msg_type = if self.is_host { 1u8 } else { 0u8 };
        buf.push(msg_type);
        buf.push(network::PROTOCOL_VERSION as u8);
        
        let paddle_x = if self.is_host { self.host_paddle.x } else { self.client_paddle.x };
        let paddle_y = if self.is_host { self.host_paddle.y } else { self.client_paddle.y };
        buf.extend_from_slice(&paddle_x.to_le_bytes());
        buf.extend_from_slice(&paddle_y.to_le_bytes());
        
        buf.extend_from_slice(&self.puck.x.to_le_bytes());
        buf.extend_from_slice(&self.puck.y.to_le_bytes());
        buf.extend_from_slice(&self.puck.vx.to_le_bytes());
        buf.extend_from_slice(&self.puck.vy.to_le_bytes());
        
        buf.push(self.score[0] as u8);
        buf.push(self.score[1] as u8);
        buf.extend_from_slice(&self.countdown.to_le_bytes());
        
        let is_host_auth = if self.is_host { self.prev_auth } else { !self.prev_auth };
        let mut flags = 0u8;
        if is_host_auth { flags |= 0x01; }
        if self.hit > 0 { flags |= 0x02; }
        if self.wall_hit > 0 { flags |= 0x04; }
        if self.goal_scored > 0 { flags |= 0x08; }
        buf.push(flags);
        
        Some(buf)
    }

    fn apply_net(&mut self, msg: &[u8]) {
        if msg.len() < 33 { return; }

        if msg[1] as u32 != network::PROTOCOL_VERSION {
            self.version_mismatch = true;
            return;
        }

        let msg_type = msg[0];
        let mut b = [0u8; 4];
        let mut read_f32 = |offset: usize| {
            b.copy_from_slice(&msg[offset..offset+4]);
            f32::from_le_bytes(b)
        };

        let px = read_f32(2);
        let py = read_f32(6);
        let puck_x = read_f32(10);
        let puck_y = read_f32(14);
        let puck_vx = read_f32(18);
        let puck_vy = read_f32(22);
        let recv_score = [msg[26] as u32, msg[27] as u32];
        let countdown_val = read_f32(28);
        
        let flags = msg[32];
        let is_host_auth = (flags & 0x01) != 0;
        let recv_hit = (flags & 0x02) != 0;
        let recv_wall_hit = (flags & 0x04) != 0;
        let recv_goal_scored = (flags & 0x08) != 0;

        let auth_changed = is_host_auth != self.prev_recv_host_auth;
        self.prev_recv_host_auth = is_host_auth;

        let recv_sum = recv_score[0] + recv_score[1];
        let local_sum = self.score[0] + self.score[1];
        let fresh_round = recv_sum >= local_sum;

        if self.is_host && msg_type == 0 {
            self.target_opponent = [px, py];
            self.score[0] = self.score[0].max(recv_score[0]);
            self.score[1] = self.score[1].max(recv_score[1]);

            if !is_host_auth || auth_changed {
                if fresh_round {
                    self.target_puck = Puck { x: puck_x, y: puck_y, vx: puck_vx, vy: puck_vy };
                    if countdown_val > self.countdown + 1.0 || countdown_val < self.countdown {
                        self.countdown = countdown_val;
                    }
                }
            }
            if recv_hit { self.hit = 1; }
            if recv_wall_hit { self.wall_hit = 1; }
            if recv_goal_scored { self.goal_scored = 1; }
        } else if !self.is_host && msg_type == 1 {
            self.target_opponent = [px, py];
            if fresh_round {
                self.score[0] = self.score[0].max(recv_score[0]);
                self.score[1] = self.score[1].max(recv_score[1]);
            }
            if is_host_auth || auth_changed {
                if fresh_round {
                    self.target_puck = Puck { x: puck_x, y: puck_y, vx: puck_vx, vy: puck_vy };
                    if countdown_val > self.countdown + 1.0 || countdown_val < self.countdown {
                        self.countdown = countdown_val;
                    }
                }
            }
            if recv_hit { self.hit = 1; }
            if recv_wall_hit { self.wall_hit = 1; }
            if recv_goal_scored { self.goal_scored = 1; }
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
                            let _ = sock.send_to(&msg, peer).await;
                        }
                    }
                } else {
                    let mut socket_guard = webrtc_socket.lock().await;
                    let peer_guard = webrtc_peer_id.lock().await;
                    if let (Some(socket), Some(peer)) = (socket_guard.as_mut(), peer_guard.as_ref()) {
                        if let Ok(channel) = socket.get_channel_mut(0) {
                            let packet = Packet::from(msg);
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
        let gs = create_test_gamestate(true, false);
        let msg = gs.net_msg().expect("Should produce a message");
        assert_eq!(msg.len(), 33);
        assert_eq!(msg[0], 1); // type = state
        assert_eq!(msg[1], network::PROTOCOL_VERSION as u8);
    }

    #[test]
    fn test_client_net_msg_format() {
        let gs = create_test_gamestate(false, false);
        let msg = gs.net_msg().expect("Should produce a message");
        assert_eq!(msg.len(), 33);
        assert_eq!(msg[0], 0); // type = input
        assert_eq!(msg[1], network::PROTOCOL_VERSION as u8);
    }

    #[test]
    fn test_single_player_no_net_msg() {
        let gs = create_test_gamestate(true, true);
        assert!(gs.net_msg().is_none());
    }

    fn build_test_packet(msg_type: u8, px: f32, py: f32, puck_x: f32, puck_y: f32, vx: f32, vy: f32, s0: u8, s1: u8, count: f32, flags: u8) -> Vec<u8> {
        let mut buf = vec![msg_type, network::PROTOCOL_VERSION as u8];
        buf.extend_from_slice(&px.to_le_bytes());
        buf.extend_from_slice(&py.to_le_bytes());
        buf.extend_from_slice(&puck_x.to_le_bytes());
        buf.extend_from_slice(&puck_y.to_le_bytes());
        buf.extend_from_slice(&vx.to_le_bytes());
        buf.extend_from_slice(&vy.to_le_bytes());
        buf.push(s0);
        buf.push(s1);
        buf.extend_from_slice(&count.to_le_bytes());
        buf.push(flags);
        buf
    }

    #[test]
    fn test_apply_net_parses_host_state() {
        let mut gs = create_test_gamestate(false, false); // client
        // type=1(state), paddle=(120, 150), puck=(180, 180), v=(10, -10), score=[1,0], count=0, flags=1(isHostAuth)
        let msg = build_test_packet(1, 120.0, 150.0, 180.0, 180.0, 10.0, -10.0, 1, 0, 0.0, 1);
        
        gs.apply_net(&msg);
        
        assert_eq!(gs.target_opponent[0], 120.0);
        assert_eq!(gs.target_opponent[1], 150.0);
        assert_eq!(gs.score[0], 1);
        assert_eq!(gs.countdown, 0.0);
    }

    #[test]
    fn test_apply_net_parses_client_input() {
        let mut gs = create_test_gamestate(true, false); // host
        // type=0(input), paddle=(150, 100), flags=0(!isHostAuth)
        let msg = build_test_packet(0, 150.0, 100.0, 50.0, 60.0, 10.0, -5.0, 0, 1, 1.5, 0);
        
        gs.apply_net(&msg);
        
        assert_eq!(gs.target_opponent[0], 150.0);
        assert_eq!(gs.target_opponent[1], 100.0);
        assert_eq!(gs.score[1], 1);
        assert_eq!(gs.countdown, 1.5);
    }

    #[test]
    fn test_apply_net_rejects_malformed() {
        let mut gs = create_test_gamestate(true, false);
        let initial_score = gs.score;
        gs.apply_net(b"short packet");
        assert_eq!(gs.score, initial_score);
    }

    #[test]
    fn test_apply_net_score_max_applies() {
        let mut gs = create_test_gamestate(true, false);
        gs.score = [2, 1]; // Local score
        
        // type=0, remote has [1, 3]
        let msg = build_test_packet(0, 100.0, 100.0, 0.0, 0.0, 0.0, 0.0, 1, 3, 0.0, 1);
        gs.apply_net(&msg);
        
        // Should take max: [2, 3]
        assert_eq!(gs.score[0], 2);
        assert_eq!(gs.score[1], 3);
    }

    #[test]
    fn test_stale_packet_guard() {
        let mut gs = create_test_gamestate(false, false);
        gs.score = [1, 0]; // Local already scored
        gs.countdown = 2.5; // Post-goal countdown
        
        // Stale packet from before goal (score sum 0 < 1)
        let stale_msg = build_test_packet(1, 100.0, 200.0, 50.0, 60.0, 10.0, -5.0, 0, 0, 0.0, 0);
        
        gs.apply_net(&stale_msg);
        
        // Countdown should NOT be overwritten by stale packet
        assert_eq!(gs.countdown, 2.5);
    }

    #[test]
    fn test_authority_detection() {
        let mut gs_host = create_test_gamestate(true, false);
        let mut gs_client = create_test_gamestate(false, false);
        
        gs_host.puck.y = TH - 100.0;
        gs_client.puck.y = TH - 100.0;
        assert!(gs_host.auth());
        assert!(!gs_client.auth());
        
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
