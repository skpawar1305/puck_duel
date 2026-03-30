/// Game configuration constants
///
/// These values control physics, rendering, and gameplay balance.
/// Tuning guide:
/// - Increase FRICTION for slower, more controlled gameplay
/// - Increase WALL_REST for bouncier walls
/// - Increase AUTH_HYSTERESIS for more stable authority handoffs
/// - Increase MAX_SPEED to allow faster shots

/// Table dimensions (logical pixels, scaled to fit screen)
pub const TABLE_WIDTH: f32 = 360.0;
pub const TABLE_HEIGHT: f32 = 640.0;

/// Puck radius
pub const PUCK_RADIUS: f32 = 20.0;

/// Paddle radius
pub const PADDLE_RADIUS: f32 = 27.0;

/// Goal width (center gap where puck can score)
pub const GOAL_WIDTH: f32 = 110.0;

/// Corner fillet radius (rounded corners of the table)
pub const CORNER_RADIUS: f32 = 42.0;

/// Maximum puck speed (prevents tunneling through walls)
pub const MAX_SPEED: f32 = 990.0;

/// Wall restitution coefficient (1.0 = perfectly elastic, 0.0 = no bounce)
pub const WALL_REST: f32 = 0.88;

/// Friction coefficient applied to puck velocity each frame
/// Higher = more friction = puck slows faster
pub const FRICTION: f32 = 0.22;

/// Authority hysteresis band around midline (pixels)
/// Prevents rapid authority flipping when puck is near center
pub const AUTH_HYSTERESIS: f32 = 12.0;

/// Winning score (first to reach this wins)
pub const WINNING_SCORE: u32 = 6;

/// Countdown duration before game starts (seconds)
pub const COUNTDOWN_DURATION: f32 = 3.0;

/// Post-goal countdown before play resumes (seconds)
pub const GOAL_COUNTDOWN: f32 = 2.5;

/// Dead reckoning blend factor (how quickly non-auth peer interpolates)
/// Higher = snappier correction, lower = smoother but more lag
pub const DEAD_RECKONING_BLEND: f32 = 0.28;

/// Velocity blend factor for dead reckoning
pub const DEAD_RECKONING_VELOCITY_BLEND: f32 = 0.40;

/// Error threshold for instant snap (pixels)
/// If dead-reckoned position differs by more than this, snap immediately
pub const DEAD_RECKONING_SNAP_THRESHOLD: f32 = 120.0;

/// Near-miss detection zone (pixels from goal line)
pub const NEAR_MISS_ZONE: f32 = 12.0;

/// Near-miss cooldown (milliseconds)
pub const NEAR_MISS_COOLDOWN_MS: u64 = 2000;

/// AI difficulty settings
pub mod ai {
    /// Chase speed when puck is behind AI (pixels/frame)
    pub const CHASE_SPEED: f32 = 12.0;

    /// Intercept speed when puck is approaching
    pub const INTERCEPT_SPEED: f32 = 8.0;

    /// Return-to-center speed when puck is in opponent's half
    pub const RETURN_SPEED: f32 = 3.5;

    /// How far in front of puck AI positions itself (pixels)
    pub const BLOCK_DISTANCE: f32 = 50.0;

    /// Home position Y when returning to center
    pub const HOME_Y: f32 = 110.0;
}

/// Network configuration
pub mod network {
    /// Broadcast channel capacity for received messages
    pub const MSG_CHANNEL_CAPACITY: usize = 64;

    /// Socket polling interval (milliseconds)
    pub const SOCKET_POLL_INTERVAL_MS: u64 = 50;

    /// Game loop target FPS
    pub const TARGET_FPS: u32 = 60;
}

/// Audio cue thresholds
pub mod audio {
    /// Speed threshold for wall hit sound variation
    pub const WALL_HIT_SPEED_THRESHOLD: f32 = 300.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_dimensions() {
        // Table should be taller than wide (portrait orientation)
        assert!(TABLE_HEIGHT > TABLE_WIDTH);
        assert_eq!(TABLE_WIDTH, 360.0);
        assert_eq!(TABLE_HEIGHT, 640.0);
    }

    #[test]
    fn test_paddle_larger_than_puck() {
        // Paddle radius should be larger than puck radius for good gameplay
        assert!(PADDLE_RADIUS > PUCK_RADIUS);
    }

    #[test]
    fn test_goal_fits_on_table() {
        // Goal width should be less than table width
        assert!(GOAL_WIDTH < TABLE_WIDTH);
        // Goal should be centered, leaving equal space on both sides
        let side_space = (TABLE_WIDTH - GOAL_WIDTH) / 2.0;
        assert!(side_space > PUCK_RADIUS); // Enough space for post collision
    }

    #[test]
    fn test_physics_constants_reasonable() {
        // Friction should be between 0 and 1
        assert!((0.0..=1.0).contains(&FRICTION));
        // Wall restitution should be between 0 and 2 (0 = no bounce, 2 = super bouncy)
        assert!((0.0..=2.0).contains(&WALL_REST));
        // Max speed should be positive
        assert!(MAX_SPEED > 0.0);
    }

    #[test]
    fn test_winning_score_reasonable() {
        // Winning score should be achievable but not too short
        assert!(WINNING_SCORE >= 3);
        assert!(WINNING_SCORE <= 21);
    }
}
