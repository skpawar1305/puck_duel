import re

with open('src-tauri/src/game.rs', 'r') as f:
    content = f.read()

new_tests = """    #[test]
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
"""

# Replace anything from "    #[test]\n    fn test_host_net_msg_format" to the end of file
pattern = r'    #\[test\]\n    fn test_host_net_msg_format\(\) \{.*\}$'
new_content = re.sub(pattern, new_tests.strip(), content, flags=re.MULTILINE | re.DOTALL)

with open('src-tauri/src/game.rs', 'w') as f:
    f.write(new_content)

print("Tests patched!")
