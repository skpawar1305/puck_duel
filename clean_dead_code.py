import re

# config.rs
with open('src-tauri/src/config.rs', 'r') as f:
    text = f.read()

unused_configs = [
    r"pub const WINNING_SCORE: u32 = 6;\n",
    r"/// Countdown duration before game starts \(seconds\)\npub const COUNTDOWN_DURATION: f32 = 3\.0;\n",
    r"/// Post-goal countdown before play resumes \(seconds\)\npub const GOAL_COUNTDOWN: f32 = 2\.5;\n",
    r"/// Dead reckoning blend factor \(how quickly non-auth peer interpolates\)\n/// Higher = snappier correction, lower = smoother but more lag\npub const DEAD_RECKONING_BLEND: f32 = 0\.28;\n",
    r"/// Velocity blend factor for dead reckoning\npub const DEAD_RECKONING_VELOCITY_BLEND: f32 = 0\.40;\n",
    r"/// Near-miss detection zone \(pixels from goal line\)\npub const NEAR_MISS_ZONE: f32 = 12\.0;\n",
    r"/// Near-miss cooldown \(milliseconds\)\npub const NEAR_MISS_COOLDOWN_MS: u64 = 2000;\n",
    r"/// Broadcast channel capacity for received messages\n    pub const MSG_CHANNEL_CAPACITY: usize = 64;\n",
    r"/// Game loop target FPS\n    pub const TARGET_FPS: u32 = 60;\n",
    r"/// Base puck position lerp factor for dead reckoning\n    /// Adaptive blending will modify this based on error magnitude\n    pub const PUCK_POSITION_LERP: f32 = 0\.50;\n",
    r"/// Speed threshold for wall hit sound variation\n    pub const WALL_HIT_SPEED_THRESHOLD: f32 = 300\.0;\n"
]

for pat in unused_configs:
    text = re.sub(pat, "", text)

with open('src-tauri/src/config.rs', 'w') as f:
    f.write(text)

# transport.rs
with open('src-tauri/src/transport.rs', 'r') as f:
    t_text = f.read()

t_text = re.sub(r"    /// Get a reference to the msg_tx channel\n    pub fn get_msg_tx\(&self\) -> &broadcast::Sender<Vec<u8>> \{\n        &self\.msg_tx\n    \}\n", "", t_text)
t_text = re.sub(r"/// Send a raw string message on the active WebRTC data channel\.\n/// Returns `true` if the send succeeded\.\npub async fn send_msg\(.*?\}\n\}\n", "", t_text, flags=re.MULTILINE | re.DOTALL)

with open('src-tauri/src/transport.rs', 'w') as f:
    f.write(t_text)

# udp_transport.rs
with open('src-tauri/src/udp_transport.rs', 'r') as f:
    u_text = f.read()

u_text = re.sub(r"/// Internal helper used by the game engine, not exposed as a Tauri command\.\n///\n/// Returns `true` on success \(message sent\) and `false` otherwise\.\npub async fn send_msg_internal\(.*?\}\n\}\n", "", u_text, flags=re.MULTILINE | re.DOTALL)

with open('src-tauri/src/udp_transport.rs', 'w') as f:
    f.write(u_text)

# physics.rs
with open('src-tauri/src/physics.rs', 'r') as f:
    p_text = f.read()
p_text = re.sub(r"    pub fn new\(x: f32, y: f32, vx: f32, vy: f32\) -> Self \{\n.*?\n    \}\n\n", "", p_text, flags=re.MULTILINE|re.DOTALL)
p_text = re.sub(r"    pub fn speed\(&self\) -> f32 \{\n.*?\n    \}\n\n", "", p_text, flags=re.MULTILINE|re.DOTALL)
p_text = re.sub(r"    pub fn new\(x: f32, y: f32\) -> Self \{\n.*?\n    \}\n\n", "", p_text, flags=re.MULTILINE|re.DOTALL)
p_text = re.sub(r"/// Apply simple friction to puck\.\npub fn apply_friction\(puck: &mut Puck, dt: f32\) \{.*?\}\n", "", p_text, flags=re.MULTILINE | re.DOTALL)
p_text = re.sub(r"/// Clamp puck speed to MAX_SPEED\.\npub fn clamp_max_speed\(puck: &mut Puck\) \{.*?\}\n", "", p_text, flags=re.MULTILINE | re.DOTALL)

with open('src-tauri/src/physics.rs', 'w') as f:
    f.write(p_text)

print("Swept dead code!")
