import re
with open('src-tauri/src/udp_transport.rs', 'r') as f: text = f.read()
text = re.sub(
r"                    if let Ok\(msg\) = String::from_utf8\(buf\[\.\.len\]\.to_vec\(\)\) \{\n                        let _ = app\.emit\(\"udp-msg-received\", \(addr\.to_string\(\), msg\.clone\(\)\)\);\n                        let _ = msg_tx\.send\(msg\); // also broadcast into channel\n                    \}",
r"                    let packet = buf[..len].to_vec();\n                    let _ = msg_tx.send(packet);", text, flags=re.MULTILINE|re.DOTALL)
with open('src-tauri/src/udp_transport.rs', 'w') as f: f.write(text)

with open('src-tauri/src/transport.rs', 'r') as f: text = f.read()
text = re.sub(
r"                            if let Ok\(msg\) = String::from_utf8\(packet\.to_vec\(\)\) \{\n                                let _ = msg_tx\.send\(msg\);\n                            \} else \{\n                                warn!\(\"Received invalid UTF-8 network packet\"\);\n                            \}",
r"                            let _ = msg_tx.send(packet.to_vec());", text, flags=re.MULTILINE|re.DOTALL)
with open('src-tauri/src/transport.rs', 'w') as f: f.write(text)
