use serde::{Serialize, Deserialize};
use crate::config::*;
use crate::physics::*;

pub const TW: f32 = TABLE_WIDTH;
pub const TH: f32 = TABLE_HEIGHT;
pub const PR: f32 = PUCK_RADIUS;
pub const PAR: f32 = PADDLE_RADIUS;
pub const GOAL_W: f32 = GOAL_WIDTH;
pub const GX: f32 = (TW - GOAL_W) / 2.0;
pub const CR: f32 = CORNER_RADIUS;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RenderState {
    pub puck:          [f32; 2],
    pub puck_speed:    f32,
    pub host_paddle:   [f32; 2],
    pub client_paddle: [f32; 2],
    pub score:         [u32; 2],
    pub wall_flash:    f32,
    pub goal_flash:    f32,
    pub score_flash:   [f32; 2],
    pub hit:           u8,
    pub wall_hit:      u8,
    pub goal_scored:   u8,
    pub countdown:     f32,
    pub game_over:     bool,
}

pub struct GameState {
    pub puck:          Puck,
    pub host_paddle:   Paddle,
    pub client_paddle: Paddle,
    pub score:         [u32; 2],
    pub wall_flash:    f32,
    pub goal_flash:    f32,
    pub score_flash:   [f32; 2],
    pub hit:           u8,
    pub wall_hit:      u8,
    pub goal_scored:   u8,
    pub countdown:     f32,
    pub game_over:     bool,
}

impl GameState {
    pub fn new() -> Self {
        Self {
            puck:          Puck::new(TW / 2.0, TH / 2.0, 0.0, 0.0),
            host_paddle:   Paddle::new(TW / 2.0, TH - 120.0),
            client_paddle: Paddle::new(TW / 2.0, 120.0),
            score:         [0, 0],
            wall_flash:    0.0,
            goal_flash:    0.0,
            score_flash:   [0.0; 2],
            hit:           0,
            wall_hit:      0,
            goal_scored:   0,
            countdown:     COUNTDOWN_DURATION,
            game_over:     false,
        }
    }

    fn reset_puck(&mut self, loser: Option<usize>) {
        match loser {
            Some(0) => {
                self.puck = Puck::new(TW / 2.0, TH * 0.75, 0.0, 0.0);
            }
            Some(1) => {
                self.puck = Puck::new(TW / 2.0, TH * 0.25, 0.0, 0.0);
            }
            _ => {
                self.puck = Puck::new(TW / 2.0, TH / 2.0, 0.0, 0.0);
            }
        }
    }

    /// Server-side update: takes both paddle positions directly and runs physics authoritatively.
    pub fn server_update(&mut self, dt: f32, host_ptr: [f32; 2], client_ptr: [f32; 2]) {
        self.hit = 0;
        self.wall_hit = 0;
        self.goal_scored = 0;

        // Countdown
        if self.countdown > 0.0 {
            self.countdown -= dt;
            if self.countdown < 0.0 { self.countdown = 0.0; }
        }

        // Update host paddle (bottom)
        self.host_paddle.x = host_ptr[0].clamp(PAR, TW - PAR);
        self.host_paddle.y = host_ptr[1].clamp(TH / 2.0 + PAR / 2.0, TH - PAR);
        self.host_paddle.pvx = host_ptr[0] - self.host_paddle.x;
        self.host_paddle.pvy = host_ptr[1] - self.host_paddle.y;

        // Update client paddle (top)
        self.client_paddle.x = client_ptr[0].clamp(PAR, TW - PAR);
        self.client_paddle.y = client_ptr[1].clamp(PAR, TH / 2.0 - PAR / 2.0);
        self.client_paddle.pvx = client_ptr[0] - self.client_paddle.x;
        self.client_paddle.pvy = client_ptr[1] - self.client_paddle.y;

        // Physics substeps (120Hz)
        let substeps = 4;
        let sub_dt = dt / (substeps as f32);
        for _ in 0..substeps {
            self.puck.x += self.puck.vx * sub_dt;
            self.puck.y += self.puck.vy * sub_dt;

            apply_friction(&mut self.puck, sub_dt);

            // Corner collisions
            let (px, py) = (self.puck.x, self.puck.y);
            collide_corner_puck(&mut self.puck, CR, CR, px < CR && py < CR);
            collide_corner_puck(&mut self.puck, TW - CR, CR, px > TW - CR && py < CR);
            collide_corner_puck(&mut self.puck, CR, TH - CR, px < CR && py > TH - CR);
            collide_corner_puck(&mut self.puck, TW - CR, TH - CR, px > TW - CR && py > TH - CR);

            // Side walls
            if self.puck.x - PR < 0.0 {
                self.puck.x = PR;
                self.puck.vx = -self.puck.vx * WALL_REST;
            } else if self.puck.x + PR > TW {
                self.puck.x = TW - PR;
                self.puck.vx = -self.puck.vx * WALL_REST;
            }

            // End walls with goal gap
            let in_gap = self.puck.x > GX && self.puck.x < GX + GOAL_W;
            if self.puck.y - PR < 0.0 && !in_gap {
                self.puck.y = PR;
                self.puck.vy = -self.puck.vy * WALL_REST;
            } else if self.puck.y + PR > TH && !in_gap {
                self.puck.y = TH - PR;
                self.puck.vy = -self.puck.vy * WALL_REST;
            }

            // Goal posts
            collide_goal_post(&mut self.puck, GX, 0.0);
            collide_goal_post(&mut self.puck, GX + GOAL_W, 0.0);
            collide_goal_post(&mut self.puck, GX, TH);
            collide_goal_post(&mut self.puck, GX + GOAL_W, TH);

            // Paddle collisions
            if collide_paddle_puck(&mut self.puck, &self.host_paddle) { self.hit = 1; }
            if collide_paddle_puck(&mut self.puck, &self.client_paddle) { self.hit = 1; }

            clamp_max_speed(&mut self.puck);
        }

        // Goal detection
        if self.countdown <= 0.0 {
            if self.puck.y < 0.0 {
                self.score[0] += 1;
                self.goal_flash = 1.0;
                self.score_flash[0] = 1.0;
                self.goal_scored = 1;
                if self.score[0] >= WINNING_SCORE {
                    self.game_over = true;
                } else {
                    self.reset_puck(Some(1));
                    self.countdown = GOAL_COUNTDOWN;
                }
            } else if self.puck.y > TH {
                self.score[1] += 1;
                self.goal_flash = 1.0;
                self.score_flash[1] = 1.0;
                self.goal_scored = 1;
                if self.score[1] >= WINNING_SCORE {
                    self.game_over = true;
                } else {
                    self.reset_puck(Some(0));
                    self.countdown = GOAL_COUNTDOWN;
                }
            }
        }

        // Decay FX
        self.goal_flash  = (self.goal_flash - dt * 2.5).max(0.0);
        self.wall_flash  = (self.wall_flash - dt * 7.0).max(0.0);
        self.score_flash[0] = (self.score_flash[0] - dt * 1.8).max(0.0);
        self.score_flash[1] = (self.score_flash[1] - dt * 1.8).max(0.0);
    }

    pub fn to_render(&self) -> RenderState {
        RenderState {
            puck:          [self.puck.x, self.puck.y],
            puck_speed:    self.puck.speed(),
            host_paddle:   [self.host_paddle.x, self.host_paddle.y],
            client_paddle: [self.client_paddle.x, self.client_paddle.y],
            score:         self.score,
            wall_flash:    self.wall_flash,
            goal_flash:    self.goal_flash,
            score_flash:   self.score_flash,
            hit:           self.hit,
            wall_hit:      self.wall_hit,
            goal_scored:   self.goal_scored,
            countdown:     self.countdown,
            game_over:     self.game_over,
        }
    }
}
