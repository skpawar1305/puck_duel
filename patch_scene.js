const fs = require('fs');
let code = fs.readFileSync('src/lib/components/Scene.svelte', 'utf-8');

// 1. Add authoritative state & velocity tracking
code = code.replace(
    `let targetHostPaddlePos = { x: 0, y: 0.2, z: 4 };`,
    `let targetHostPaddlePos = { x: 0, y: 0.2, z: 4 };\n    let targetPuckVel = { x: 0, y: 0, z: 0 };\n    \n    let puckZ = $derived(puckPos[2] || 0);\n    let amIAuthoritative = $derived(isSinglePlayer ? true : (isHost ? puckZ >= 0 : puckZ < 0));\n    let wasAuthoritative = true;\n\n    $effect(() => {\n        if (puckRigidBody && amIAuthoritative !== wasAuthoritative) {\n            wasAuthoritative = amIAuthoritative;\n            if (amIAuthoritative) {\n                puckRigidBody.setLinvel(targetPuckVel, true);\n            }\n        }\n    });`
);

// 2. Modify message reception for Host
const hostRecvReplace = `if (isHost) {
                        if (msg.type === "input") {
                            targetClientPaddlePos = {
                                x: msg.pos[0],
                                y: 0.2,
                                z: msg.pos[2],
                            };
                        }`;
const hostRecvWithPuck = `if (isHost) {
                        if (msg.type === "input") {
                            targetClientPaddlePos = {
                                x: msg.pos[0],
                                y: 0.2,
                                z: msg.pos[2],
                            };
                            if (msg.puck) {
                                targetPuckPos = { x: msg.puck[0], y: 0.1, z: msg.puck[2] };
                                if (msg.vel) targetPuckVel = { x: msg.vel[0], y: msg.vel[1], z: msg.vel[2] };
                                if (msg.score) score = msg.score;
                                
                                if (puckRigidBody && !amIAuthoritative) {
                                    const curPuck = puckRigidBody.translation();
                                    if (Math.abs(curPuck.z - msg.puck[2]) > 2) {
                                        puckRigidBody.setTranslation(targetPuckPos, true);
                                    }
                                }
                            }
                        }`;
code = code.replace(hostRecvReplace, hostRecvWithPuck);

// 3. Modify message reception for Client
const clientRecvReplace = `if (msg.type === "state") {
                            score = msg.score;
                            targetPuckPos = {
                                x: msg.puck[0],
                                y: 0.1,
                                z: msg.puck[2],
                            };
                            targetHostPaddlePos = {
                                x: msg.hostPaddle[0],
                                y: 0.2,
                                z: msg.hostPaddle[2],
                            };

                            if (puckRigidBody) {
                                const curPuck = puckRigidBody.translation();
                                if (Math.abs(curPuck.z - msg.puck[2]) > 2) {
                                    puckRigidBody.setTranslation(
                                        targetPuckPos,
                                        true,
                                    );
                                }
                            }
                        }`;

const clientRecvWithVel = `if (msg.type === "state") {
                            if (msg.score) score = msg.score;
                            if (msg.hostPaddle) targetHostPaddlePos = { x: msg.hostPaddle[0], y: 0.2, z: msg.hostPaddle[2] };
                            if (msg.puck) {
                                targetPuckPos = { x: msg.puck[0], y: 0.1, z: msg.puck[2] };
                                if (msg.vel) targetPuckVel = { x: msg.vel[0], y: msg.vel[1], z: msg.vel[2] };
                                
                                if (puckRigidBody && !amIAuthoritative) {
                                    const curPuck = puckRigidBody.translation();
                                    if (Math.abs(curPuck.z - msg.puck[2]) > 2) {
                                        puckRigidBody.setTranslation(targetPuckPos, true);
                                    }
                                }
                            }
                        }`;
code = code.replace(clientRecvReplace, clientRecvWithVel);

fs.writeFileSync('src/lib/components/Scene.svelte', code);
