/// Physics module for puck collision and movement
///
/// This module contains testable physics functions for the game.
use crate::config::*;

/// Internal puck representation for physics calculations
#[derive(Clone, Debug, PartialEq)]
pub struct Puck {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
}

/// Internal paddle representation for physics calculations
#[derive(Clone, Debug)]
pub struct Paddle {
    pub x: f32,
    pub y: f32,
    pub pvx: f32,
    pub pvy: f32,
}

impl Puck {
    pub fn new(x: f32, y: f32, vx: f32, vy: f32) -> Self {
        Self { x, y, vx, vy }
    }

    pub fn speed(&self) -> f32 {
        (self.vx * self.vx + self.vy * self.vy).sqrt()
    }
}

impl Paddle {
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            pvx: 0.0,
            pvy: 0.0,
        }
    }
}

/// Detect and resolve collision between paddle and puck
/// Returns true if collision occurred
pub fn collide_paddle_puck(puck: &mut Puck, pad: &Paddle) -> bool {
    let dx = puck.x - pad.x;
    let dy = puck.y - pad.y;
    let d = (dx * dx + dy * dy).sqrt();
    let md = PUCK_RADIUS + PADDLE_RADIUS;

    if d >= md || d < 0.001 {
        return false;
    }

    let (nx, ny) = (dx / d, dy / d);
    puck.x = pad.x + nx * (md + 1.0);
    puck.y = pad.y + ny * (md + 1.0);

    // Relative velocity at collision point
    let rel_vx = puck.vx - pad.pvx;
    let rel_vy = puck.vy - pad.pvy;
    let dot = rel_vx * nx + rel_vy * ny;
    
    if dot < 0.0 {
        // Reflect and add power
        puck.vx = puck.vx - dot * nx * (1.0 + WALL_REST) + pad.pvx * PADDLE_POWER;
        puck.vy = puck.vy - dot * ny * (1.0 + WALL_REST) + pad.pvy * PADDLE_POWER;
        
        // Ensure minimum hit speed for satisfying shots
        let speed = (puck.vx * puck.vx + puck.vy * puck.vy).sqrt();
        if speed < MIN_HIT_SPEED {
            let scale = MIN_HIT_SPEED / speed.max(0.001);
            puck.vx *= scale;
            puck.vy *= scale;
        }
        
        true
    } else {
        false
    }
}

/// Detect and resolve collision between corner fillet and puck
/// Returns true if collision occurred
pub fn collide_corner_puck(puck: &mut Puck, cx: f32, cy: f32, in_zone: bool) -> bool {
    if !in_zone {
        return false;
    }

    let dx = puck.x - cx;
    let dy = puck.y - cy;
    let d = (dx * dx + dy * dy).sqrt();
    let max_d = (CORNER_RADIUS - 2.0) - PUCK_RADIUS;

    if d <= max_d || d < 0.001 {
        return false;
    }

    let (nx, ny) = (dx / d, dy / d);
    puck.x = cx + nx * max_d;
    puck.y = cy + ny * max_d;

    let dot = puck.vx * nx + puck.vy * ny;
    if dot > 0.0 {
        puck.vx -= dot * (1.0 + WALL_REST) * nx;
        puck.vy -= dot * (1.0 + WALL_REST) * ny;
        true
    } else {
        false
    }
}

/// Detect and resolve collision between goal post and puck
/// Returns true if collision occurred
pub fn collide_goal_post(puck: &mut Puck, px: f32, py: f32) -> bool {
    let dx = puck.x - px;
    let dy = puck.y - py;
    let d = (dx * dx + dy * dy).sqrt();

    if d >= PUCK_RADIUS || d < 0.001 {
        return false;
    }

    let (nx, ny) = (dx / d, dy / d);
    puck.x = px + nx * PUCK_RADIUS;
    puck.y = py + ny * PUCK_RADIUS;

    let dot = puck.vx * nx + puck.vy * ny;
    if dot < 0.0 {
        puck.vx -= (1.0 + WALL_REST) * dot * nx;
        puck.vy -= (1.0 + WALL_REST) * dot * ny;
        true
    } else {
        false
    }
}

/// Apply friction to puck velocity
pub fn apply_friction(puck: &mut Puck, dt: f32) {
    let sp = puck.speed();
    if sp > 0.0 {
        let loss = (FRICTION * sp * dt).min(sp);
        puck.vx -= puck.vx / sp * loss;
        puck.vy -= puck.vy / sp * loss;
    }
}

/// Clamp puck speed to maximum
pub fn clamp_max_speed(puck: &mut Puck) {
    let cs = puck.speed();
    if cs > MAX_SPEED {
        puck.vx = puck.vx / cs * MAX_SPEED;
        puck.vy = puck.vy / cs * MAX_SPEED;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_puck_speed() {
        let puck = Puck::new(0.0, 0.0, 3.0, 4.0);
        assert!((puck.speed() - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_collide_paddle_puck_no_collision() {
        let mut puck = Puck::new(100.0, 100.0, 0.0, 0.0);
        let pad = Paddle::new(200.0, 200.0);
        assert!(!collide_paddle_puck(&mut puck, &pad));
    }

    #[test]
    fn test_collide_paddle_puck_detects_collision() {
        // Place puck exactly at collision distance
        let overlap = 5.0;
        let min_dist = PUCK_RADIUS + PADDLE_RADIUS - overlap;
        let mut puck = Puck::new(0.0, min_dist, 0.0, -10.0);
        let pad = Paddle::new(0.0, 0.0);

        assert!(collide_paddle_puck(&mut puck, &pad));
        // Puck should be pushed out of collision
        assert!(puck.y > min_dist);
    }

    #[test]
    fn test_collide_paddle_puck_reflects_velocity() {
        let mut puck = Puck::new(0.0, PUCK_RADIUS + PADDLE_RADIUS - 1.0, 0.0, -100.0);
        let pad = Paddle::new(0.0, 0.0);

        collide_paddle_puck(&mut puck, &pad);

        // Velocity should be reflected (now positive)
        assert!(puck.vy > 0.0);
    }

    #[test]
    fn test_collide_corner_puck_outside_zone() {
        let mut puck = Puck::new(10.0, 10.0, 0.0, 0.0);
        assert!(!collide_corner_puck(&mut puck, 0.0, 0.0, false));
    }

    #[test]
    fn test_collide_corner_puck_no_collision() {
        let mut puck = Puck::new(100.0, 100.0, 0.0, 0.0);
        assert!(!collide_corner_puck(
            &mut puck,
            CORNER_RADIUS,
            CORNER_RADIUS,
            true
        ));
    }

    #[test]
    fn test_collide_goal_post_no_collision() {
        let mut puck = Puck::new(100.0, 100.0, 0.0, 0.0);
        assert!(!collide_goal_post(&mut puck, 0.0, 0.0));
    }

    #[test]
    fn test_collide_goal_post_detects_collision() {
        // Place puck very close to post
        let mut puck = Puck::new(PUCK_RADIUS - 1.0, 0.0, -50.0, 0.0);

        assert!(collide_goal_post(&mut puck, 0.0, 0.0));
        // Puck should be pushed away from post
        assert!(puck.x > PUCK_RADIUS - 1.0);
    }

    #[test]
    fn test_apply_friction_reduces_speed() {
        let mut puck = Puck::new(0.0, 0.0, 100.0, 0.0);
        let initial_speed = puck.speed();

        apply_friction(&mut puck, 0.016);

        assert!(puck.speed() < initial_speed);
    }

    #[test]
    fn test_apply_friction_stationary_puck() {
        let mut puck = Puck::new(0.0, 0.0, 0.0, 0.0);

        apply_friction(&mut puck, 0.016);

        assert_eq!(puck.speed(), 0.0);
    }

    #[test]
    fn test_clamp_max_speed_reduces_velocity() {
        let mut puck = Puck::new(0.0, 0.0, MAX_SPEED + 100.0, 0.0);

        clamp_max_speed(&mut puck);

        assert!((puck.speed() - MAX_SPEED).abs() < 0.001);
    }

    #[test]
    fn test_clamp_max_speed_no_change_when_under() {
        let mut puck = Puck::new(0.0, 0.0, 100.0, 0.0);
        let initial_speed = puck.speed();

        clamp_max_speed(&mut puck);

        assert!((puck.speed() - initial_speed).abs() < 0.001);
    }

    #[test]
    fn test_goal_post_positions() {
        // Verify goal post positions are at table edges
        let goal_x_left = (TABLE_WIDTH - GOAL_WIDTH) / 2.0;
        let goal_x_right = goal_x_left + GOAL_WIDTH;

        assert!(goal_x_left > 0.0);
        assert!(goal_x_right < TABLE_WIDTH);
    }
}
