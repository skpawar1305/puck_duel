use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use crate::udp_server::UdpState;

// ── Constants ────────────────────────────────────────────────────────────────
const TW: f32 = 360.0;
const TH: f32 = 640.0;
const PR: f32 = 20.0;
const PAR: f32 = 36.0;
const GOAL_W: f32 = 110.0;
const GX: f32 = (TW - GOAL_W) / 2.0;
const CR: f32 = 42.0;
const MAX_SPEED: f32 = 900.0;
const MIN_SPEED: f32 = 160.0;
const WALL_REST: f32 = 0.88;
const FRICTION: f32 = 0.22;

// ── Public state managed by Tauri ─────────────────────────────────────────────
pub struct GameEngine {
    pub running: Arc<AtomicBool>,
    pub pointer: Arc<Mutex<[f32; 2]>>,
}
impl GameEngine {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
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

    // authority tracking — needed for snap-on-handoff
    prev_auth:   bool,
    is_host:     bool,
    is_single:   bool,
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
            hit:0, wall_hit:0, goal_scored:0,
            prev_auth: is_single || is_host,
            is_host, is_single,
        }
    }

    fn auth(&self) -> bool {
        self.is_single || (self.is_host == (self.puck.y >= TH / 2.0))
    }

    fn reset_puck(&mut self) {
        self.puck = Puck { x:TW/2.0, y:TH/2.0, vx:0.0, vy:0.0 };
    }

    fn update(&mut self, dt: f32, ptr: [f32; 2]) {
        self.hit = 0; self.wall_hit = 0; self.goal_scored = 0;

        // ── My paddle ────────────────────────────────────────────────────────
        {
            let p = if self.is_host { &mut self.host_paddle } else { &mut self.client_paddle };
            let (px, py) = (p.x, p.y);
            let (dx, dy) = (ptr[0] - p.x, ptr[1] - p.y);
            let pd = (dx*dx + dy*dy).sqrt();
            if pd > 0.0 {
                let s = pd.min(1400.0 * dt);
                p.x += dx/pd * s; p.y += dy/pd * s;
            }
            p.x = p.x.max(PAR).min(TW - PAR);
            if self.is_host { p.y = p.y.max(TH/2.0 + PAR/2.0).min(TH - PAR); }
            else             { p.y = p.y.max(PAR).min(TH/2.0 - PAR/2.0); }
            p.pvx = (p.x - px) / dt;
            p.pvy = (p.y - py) / dt;
        }

        // ── Opponent paddle ───────────────────────────────────────────────────
        if self.is_single {
            let ai = &mut self.client_paddle;
            let (apx, apy) = (ai.x, ai.y);
            let aggressive = self.puck.y < TH/2.0 || self.puck.vy < -50.0;
            let spd: f32 = if aggressive { 5.5 } else { 1.8 };
            let tgt_y: f32 = if aggressive {
                (self.puck.y + 50.0).max(PAR).min(TH/2.0 - PAR)
            } else { 110.0 };
            ai.x += (self.puck.x - ai.x) * (spd * dt).min(1.0);
            ai.y += (tgt_y - ai.y) * (spd * dt * 1.3).min(1.0);
            ai.x = ai.x.max(PAR).min(TW - PAR);
            ai.y = ai.y.max(PAR).min(TH/2.0 - PAR/2.0);
            ai.pvx = (ai.x - apx) / dt;
            ai.pvy = (ai.y - apy) / dt;
        } else {
            let op = if self.is_host { &mut self.client_paddle } else { &mut self.host_paddle };
            let (opx, opy) = (op.x, op.y);
            op.x += (self.target_opponent[0] - op.x) * 0.85;
            op.y += (self.target_opponent[1] - op.y) * 0.85;
            op.pvx = (op.x - opx) / dt;
            op.pvy = (op.y - opy) / dt;
        }

        // ── Puck ──────────────────────────────────────────────────────────────
        let auth_now = self.auth();

        if auth_now {
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

            // Goals
            if self.puck.y < -20.0 {
                self.score[0] += 1; self.goal_scored = 1;
                self.goal_flash = 1.0; self.score_flash[0] = 1.0; self.reset_puck();
            } else if self.puck.y > TH + 20.0 {
                self.score[1] += 1; self.goal_scored = 1;
                self.goal_flash = 1.0; self.score_flash[1] = 1.0; self.reset_puck();
            }

            // Paddle collisions
            let hp = self.host_paddle.clone();
            let cp = self.client_paddle.clone();
            if collide_paddle_puck(&mut self.puck, &hp) { self.hit = 1; }
            if collide_paddle_puck(&mut self.puck, &cp) { self.hit = 1; }

            // Speed clamp
            let cs = (self.puck.vx*self.puck.vx + self.puck.vy*self.puck.vy).sqrt();
            if cs > 0.1 && cs < MIN_SPEED { self.puck.vx=self.puck.vx/cs*MIN_SPEED; self.puck.vy=self.puck.vy/cs*MIN_SPEED; }
            else if cs > MAX_SPEED        { self.puck.vx=self.puck.vx/cs*MAX_SPEED;  self.puck.vy=self.puck.vy/cs*MAX_SPEED;  }
        } else {
            // Snap on handoff: just lost authority this frame
            if self.prev_auth {
                self.puck.x  = self.target_puck.x;  self.puck.y  = self.target_puck.y;
                self.puck.vx = self.target_puck.vx; self.puck.vy = self.target_puck.vy;
            }

            // Dead reckoning
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

            // Blend toward authoritative peer state
            let ex = self.target_puck.x - self.puck.x;
            let ey = self.target_puck.y - self.puck.y;
            let err = (ex*ex + ey*ey).sqrt();
            if err > 80.0 {
                self.puck.x=self.target_puck.x; self.puck.y=self.target_puck.y;
                self.puck.vx=self.target_puck.vx; self.puck.vy=self.target_puck.vy;
            } else if err > 1.0 {
                self.puck.x += ex*0.18; self.puck.y += ey*0.18;
                self.puck.vx += (self.target_puck.vx - self.puck.vx)*0.25;
                self.puck.vy += (self.target_puck.vy - self.puck.vy)*0.25;
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
        }
    }

    /// Network message to broadcast/send this frame (authoritative side only)
    fn net_msg(&self) -> Option<String> {
        if self.is_single { return None; }
        if self.is_host {
            let msg = serde_json::json!({
                "type": "state",
                "hostPaddle": [self.host_paddle.x, self.host_paddle.y],
                "puck": [self.puck.x, self.puck.y],
                "vel":  [self.puck.vx, self.puck.vy],
                "score": self.score
            });
            Some(msg.to_string())
        } else {
            let mut m = serde_json::json!({
                "type": "input",
                "pos": [self.client_paddle.x, self.client_paddle.y]
            });
            if self.auth() {
                m["puck"]  = serde_json::json!([self.puck.x, self.puck.y]);
                m["vel"]   = serde_json::json!([self.puck.vx, self.puck.vy]);
                m["score"] = serde_json::json!(self.score);
            }
            Some(m.to_string())
        }
    }

    fn apply_net(&mut self, msg: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(msg) else { return };
        if self.is_host && v["type"] == "input" {
            if let Some(pos) = v["pos"].as_array() {
                self.target_opponent = [pos[0].as_f64().unwrap_or(0.0) as f32,
                                        pos[1].as_f64().unwrap_or(0.0) as f32];
            }
            if let (Some(p), Some(vel)) = (v["puck"].as_array(), v["vel"].as_array()) {
                self.target_puck = Puck {
                    x:  p[0].as_f64().unwrap_or(0.0) as f32,
                    y:  p[1].as_f64().unwrap_or(0.0) as f32,
                    vx: vel[0].as_f64().unwrap_or(0.0) as f32,
                    vy: vel[1].as_f64().unwrap_or(0.0) as f32,
                };
            }
            if let Some(s) = v["score"].as_array() {
                self.score = [s[0].as_u64().unwrap_or(0) as u32,
                              s[1].as_u64().unwrap_or(0) as u32];
            }
        } else if !self.is_host && v["type"] == "state" {
            if let Some(hp) = v["hostPaddle"].as_array() {
                self.target_opponent = [hp[0].as_f64().unwrap_or(0.0) as f32,
                                        hp[1].as_f64().unwrap_or(0.0) as f32];
            }
            if let (Some(p), Some(vel)) = (v["puck"].as_array(), v["vel"].as_array()) {
                self.target_puck = Puck {
                    x:  p[0].as_f64().unwrap_or(0.0) as f32,
                    y:  p[1].as_f64().unwrap_or(0.0) as f32,
                    vx: vel[0].as_f64().unwrap_or(0.0) as f32,
                    vy: vel[1].as_f64().unwrap_or(0.0) as f32,
                };
            }
            if let Some(s) = v["score"].as_array() {
                self.score = [s[0].as_u64().unwrap_or(0) as u32,
                              s[1].as_u64().unwrap_or(0) as u32];
            }
        }
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_game(
    engine:          State<'_, GameEngine>,
    udp:             State<'_, UdpState>,
    is_host:         bool,
    is_single_player:bool,
    channel:         Channel<RenderState>,
) -> Result<(), String> {
    if engine.running.swap(true, Ordering::SeqCst) {
        return Ok(()); // already running
    }

    // Reset pointer to starting position
    *engine.pointer.lock().unwrap() = if is_host { [TW/2.0, TH-120.0] } else { [TW/2.0, 120.0] };

    // Cloneable handles passed into the async task
    let running  = engine.running.clone();
    let pointer  = engine.pointer.clone();

    // UDP sockets (Arcs, cheap to clone)
    let host_sock    = udp.host_socket.lock().unwrap().clone();
    let client_sock  = udp.client_socket.lock().unwrap().clone();
    let client_addr  = udp.client_remote_addr.lock().unwrap().clone();
    let clients_arc  = udp.connected_clients.clone();

    // Subscribe to the broadcast channel (receive task → game loop, zero socket contention)
    let mut net_rx = udp.msg_tx.subscribe();

    tokio::spawn(async move {
        let mut gs       = GameState::new(is_host, is_single_player);
        let mut interval = tokio::time::interval(Duration::from_nanos(16_666_667));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last     = Instant::now();
        let mut net_tick = 0u8;

        while running.load(Ordering::Relaxed) {
            interval.tick().await;
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

            // Send UDP every 3rd frame (~20 Hz) — physics still at 60 Hz
            net_tick = net_tick.wrapping_add(1);
            if net_tick % 3 == 0 {
                if let Some(msg) = gs.net_msg() {
                    let bytes = msg.as_bytes();
                    if is_host {
                        let addrs: Vec<_> = clients_arc.lock().unwrap().keys().cloned().collect();
                        if let Some(sock) = &host_sock {
                            for addr in addrs {
                                let _ = sock.send_to(bytes, addr).await;
                            }
                        }
                    } else if let (Some(sock), Some(addr)) = (&client_sock, client_addr) {
                        let _ = sock.send_to(bytes, addr).await;
                    }
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
    engine.running.store(false, Ordering::Relaxed);
}

#[tauri::command]
pub fn set_pointer(engine: State<'_, GameEngine>, x: f32, y: f32) {
    *engine.pointer.lock().unwrap() = [x, y];
}
