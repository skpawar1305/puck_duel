/// Game configuration constants
pub const TABLE_WIDTH: f32 = 360.0;
pub const TABLE_HEIGHT: f32 = 640.0;
pub const PUCK_RADIUS: f32 = 20.0;
pub const PADDLE_RADIUS: f32 = 27.0;
pub const GOAL_WIDTH: f32 = 150.0;
pub const CORNER_RADIUS: f32 = 42.0;
pub const MAX_SPEED: f32 = 950.0;
pub const MIN_HIT_SPEED: f32 = 250.0;
pub const GOAL_POST_RADIUS: f32 = 6.0;
pub const PADDLE_SURFACE_FRICTION: f32 = 0.20;
pub const MIN_PUCK_SPEED: f32 = 2.0;
pub const PADDLE_POWER: f32 = 1.25;
pub const WALL_REST: f32 = 0.88;
pub const FRICTION: f32 = 0.06;
pub const AUTH_HYSTERESIS: f32 = 12.0;
pub const WINNING_SCORE: u32 = 6;
pub const COUNTDOWN_DURATION: f32 = 3.0;
pub const GOAL_COUNTDOWN: f32 = 2.5;
pub const DEAD_RECKONING_BLEND: f32 = 0.28;
pub const DEAD_RECKONING_VELOCITY_BLEND: f32 = 0.40;
pub const DEAD_RECKONING_SNAP_THRESHOLD: f32 = 120.0;
pub const NEAR_MISS_ZONE: f32 = 12.0;
pub const NEAR_MISS_COOLDOWN_MS: u64 = 2000;

/// AI difficulty settings
pub mod ai {
    pub const CHASE_SPEED: f32 = 11.0;
    pub const INTERCEPT_SPEED: f32 = 9.5;
    pub const RETURN_SPEED: f32 = 4.0;
    pub const BLOCK_DISTANCE: f32 = 45.0;
    pub const HOME_Y: f32 = 100.0;
    pub const REACTION_LERP: f32 = 0.35;
    pub const PREDICTION_TIME: f32 = 0.15;
    pub const DEFENSIVE_Y: f32 = 90.0;
    pub const THINK_INTERVAL: f32 = 0.085;
    pub const AIM_ERROR_X: f32 = 11.0;
    pub const AIM_ERROR_Y: f32 = 8.0;
}

/// Network configuration
pub mod network {
    pub const MSG_CHANNEL_CAPACITY: usize = 64;
    pub const SOCKET_POLL_INTERVAL_MS: u64 = 8;
    pub const TARGET_FPS: u32 = 60;
    pub const PROTOCOL_VERSION: u32 = 2;
}

/// Interpolation configuration
pub mod interpolation {
    pub const OPPONENT_PADDLE_LERP: f32 = 0.75;
    pub const PUCK_POSITION_LERP: f32 = 0.50;
    pub const PUCK_VELOCITY_LERP: f32 = 0.60;
    pub const MIN_BLEND: f32 = 0.30;
    pub const MAX_BLEND: f32 = 0.80;
    pub const ADAPTIVE_ERROR_THRESHOLD: f32 = 100.0;
    pub const HANDOFF_BLEND: f32 = 0.50;
    pub const DEAD_RECKONING_SNAP_THRESHOLD: f32 = super::DEAD_RECKONING_SNAP_THRESHOLD;
}

pub mod audio {
    pub const WALL_HIT_SPEED_THRESHOLD: f32 = 300.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_dimensions() {
        assert!(TABLE_HEIGHT > TABLE_WIDTH);
        assert_eq!(TABLE_WIDTH, 360.0);
        assert_eq!(TABLE_HEIGHT, 640.0);
    }

    #[test]
    fn test_paddle_larger_than_puck() {
        assert!(PADDLE_RADIUS > PUCK_RADIUS);
    }

    #[test]
    fn test_goal_fits_on_table() {
        assert!(GOAL_WIDTH < TABLE_WIDTH);
        let side_space = (TABLE_WIDTH - GOAL_WIDTH) / 2.0;
        assert!(side_space > PUCK_RADIUS);
    }

    #[test]
    fn test_physics_constants_reasonable() {
        assert!((0.0..=1.0).contains(&FRICTION));
        assert!((0.0..=2.0).contains(&WALL_REST));
        assert!(MAX_SPEED > 0.0);
    }

    #[test]
    fn test_winning_score_reasonable() {
        assert!(WINNING_SCORE >= 3);
        assert!(WINNING_SCORE <= 21);
    }
}
