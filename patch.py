import re

with open('src-tauri/src/game.rs', 'r') as f:
    content = f.read()

replacement = """    fn net_msg(&self) -> Option<Vec<u8>> {
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
}"""

pattern = r'    fn net_msg\(\&self\) \-\> Option\<String\> \{.*?^\}$'
new_content = re.sub(pattern, replacement, content, flags=re.MULTILINE | re.DOTALL)

with open('src-tauri/src/game.rs', 'w') as f:
    f.write(new_content)

print("Patch applied to src-tauri/src/game.rs!")
