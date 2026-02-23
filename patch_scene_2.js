const fs = require('fs');
let code = fs.readFileSync('src/lib/components/Scene.svelte', 'utf-8');

const useTaskStart = code.indexOf('useTask(() => {');
const onPointerMoveStart = code.indexOf('function onPointerMove(e: any) {');

const replacement = `useTask(() => {
        // --- PADDLE LOGIC ---
        // 1. Update Local Paddle
        if (isHost) {
            if (hostPaddleBody) {
                const t = hostPaddleBody.translation();
                const tx = pointerTarget.x;
                const tz = pointerTarget.z;
                const speedMult = 15.0;
                const vx = (tx - t.x) * speedMult;
                const vz = (tz - t.z) * speedMult;
                hostPaddleBody.setLinvel({ x: vx, y: 0, z: vz }, true);
                hostPaddleBody.setAngvel({ x: 0, y: 0, z: 0 }, true);
                if (Math.abs(t.y - 0.2) > 0.05) {
                    hostPaddleBody.setTranslation({ x: t.x, y: 0.2, z: t.z }, true);
                }
            }
            if (!isSinglePlayer && clientPaddleBody) {
                const ct = clientPaddleBody.translation();
                clientPaddleBody.setNextKinematicTranslation({
                    x: ct.x + (targetClientPaddlePos.x - ct.x) * 0.4,
                    y: 0.2,
                    z: ct.z + (targetClientPaddlePos.z - ct.z) * 0.4,
                });
                clientPaddlePos = [ct.x, 0.2, ct.z];
            } else if (isSinglePlayer && clientPaddleBody) {
                // AI Logic
                const t = clientPaddleBody.translation();
                const pTrans = puckRigidBody ? puckRigidBody.translation() : {x:0,y:0.1,z:0};
                const puckVelocity = puckRigidBody ? puckRigidBody.linvel() : {x:0,y:0,z:0};
                
                let aiSpeedMultiplier = 0.03;
                if (pTrans.z < 0 || puckVelocity.z < 0) {
                    aiSpeedMultiplier = 0.08;
                }
                
                let nextX = t.x + (pTrans.x - t.x) * aiSpeedMultiplier;
                nextX = Math.max(-2.5, Math.min(2.5, nextX));
                
                let targetZ = Math.min(-0.5, pTrans.z);
                if (pTrans.z > 0) targetZ = -4.0;
                
                let nextZ = t.z + (targetZ - t.z) * (aiSpeedMultiplier * 1.5);
                nextZ = Math.min(-0.5, Math.max(-4.5, nextZ));
                
                const speedMult = 20.0;
                const vx = (nextX - t.x) * speedMult;
                const vz = (nextZ - t.z) * speedMult;
                
                clientPaddleBody.setLinvel({ x: vx, y: 0, z: vz }, true);
                clientPaddleBody.setAngvel({ x: 0, y: 0, z: 0 }, true);
                if (Math.abs(t.y - 0.2) > 0.05) {
                    clientPaddleBody.setTranslation({ x: t.x, y: 0.2, z: t.z }, true);
                }
            }
        } else {
            // Client
            if (clientPaddleBody) {
                const t = clientPaddleBody.translation();
                const tx = pointerTarget.x;
                const tz = pointerTarget.z;
                const speedMult = 15.0;
                const vx = (tx - t.x) * speedMult;
                const vz = (tz - t.z) * speedMult;
                clientPaddleBody.setLinvel({ x: vx, y: 0, z: vz }, true);
                clientPaddleBody.setAngvel({ x: 0, y: 0, z: 0 }, true);
                if (Math.abs(t.y - 0.2) > 0.05) {
                    clientPaddleBody.setTranslation({ x: t.x, y: 0.2, z: t.z }, true);
                }
            }
            if (hostPaddleBody) {
                const t = hostPaddleBody.translation();
                const speedMult = 15.0;
                const vx = (targetHostPaddlePos.x - t.x) * speedMult;
                const vz = (targetHostPaddlePos.z - t.z) * speedMult;
                hostPaddleBody.setLinvel({ x: vx, y: 0, z: vz }, true);
                hostPaddleBody.setAngvel({ x: 0, y: 0, z: 0 }, true);
                hostPaddlePos = [t.x, 0.2, t.z];
            }
        }

        // --- PUCK LOGIC ---
        if (puckRigidBody) {
            const pTrans = puckRigidBody.translation();
            
            if (amIAuthoritative) {
                // Authoritative physics
                const hTrans = hostPaddleBody ? hostPaddleBody.translation() : {x:0, y:0.2, z:4};
                const cTrans = clientPaddleBody ? clientPaddleBody.translation() : {x:0, y:0.2, z:-4};

                // Sound
                const vel = puckRigidBody.linvel();
                const speed = Math.sqrt(vel.x * vel.x + vel.z * vel.z);
                if (speed - prevPuckSpeed > 1.5) {
                    const distH = Math.hypot(pTrans.x - hTrans.x, pTrans.z - hTrans.z);
                    const distC = Math.hypot(pTrans.x - cTrans.x, pTrans.z - cTrans.z);
                    if (distH < 1.2 || distC < 1.2) {
                        playHit();
                    } else {
                        playWall();
                    }
                }
                prevPuckSpeed = speed;

                // Physics Boundaries
                const av = puckRigidBody.angvel();
                puckRigidBody.setAngvel({ x: 0, y: av.y, z: 0 }, true);

                let py = pTrans.y, vy = vel.y;
                if (Math.abs(py - 0.1) > 0.04) { py = 0.1; vy = 0; }

                const xMax = 2.85;
                let px = pTrans.x, vx = vel.x;
                if (px < -xMax) { px = -xMax; if (vx < 0) { vx = -vx * 0.85; playWall(); } }
                if (px > xMax) { px = xMax; if (vx > 0) { vx = -vx * 0.85; playWall(); } }

                const zMax = 4.85;
                let pz = pTrans.z, vz = vel.z;
                const inGoalGap = Math.abs(px) < 1.3;
                if (!inGoalGap) {
                    if (pz < -zMax) { pz = -zMax; if (vz < 0) { vz = -vz * 0.85; playWall(); } }
                    if (pz > zMax) { pz = zMax; if (vz > 0) { vz = -vz * 0.85; playWall(); } }
                }

                const moved = Math.abs(px - pTrans.x) > 0.001 || Math.abs(py - pTrans.y) > 0.001 || Math.abs(pz - pTrans.z) > 0.001;
                if (moved) {
                    puckRigidBody.setTranslation({ x: px, y: py, z: pz }, true);
                    puckRigidBody.setLinvel({ x: vx, y: vy, z: vz }, true);
                }

                // Goal detection
                if (pTrans.z > 5.4) {
                    score[1]++;
                    playGoal();
                    puckRigidBody.setTranslation({ x: 0, y: 0.1, z: 0 }, true);
                    puckRigidBody.setLinvel({ x: 0, y: 0, z: 0 }, true);
                } else if (pTrans.z < -5.4) {
                    score[0]++;
                    playGoal();
                    puckRigidBody.setTranslation({ x: 0, y: 0.1, z: 0 }, true);
                    puckRigidBody.setLinvel({ x: 0, y: 0, z: 0 }, true);
                } else if (pTrans.y < -2) {
                    puckRigidBody.setTranslation({ x: 0, y: 0.1, z: 0 }, true);
                    puckRigidBody.setLinvel({ x: 0, y: 0, z: 0 }, true);
                }

                puckPos = [pTrans.x, 0.1, pTrans.z];
            } else {
                // Non-authoritative interpolation
                const ct = puckRigidBody.translation();
                puckRigidBody.setTranslation(
                    {
                        x: ct.x + (targetPuckPos.x - ct.x) * 0.2,
                        y: 0.1,
                        z: ct.z + (targetPuckPos.z - ct.z) * 0.2,
                    },
                    true
                );
                puckPos = [ct.x, 0.1, ct.z];
            }
        }
        
        // --- MULTIPLAYER BROADCAST ---
        if (!isSinglePlayer) {
            if (isHost) {
                const hTrans = hostPaddleBody ? hostPaddleBody.translation() : {x:0, y:0.2, z:4};
                let msgData = {
                    type: "state",
                    hostPaddle: [hTrans.x, hTrans.y, hTrans.z],
                    puck: undefined as undefined | number[],
                    vel: undefined as undefined | number[],
                    score: undefined as undefined | number[]
                };
                if (amIAuthoritative && puckRigidBody) {
                    const pTrans = puckRigidBody.translation();
                    const pVel = puckRigidBody.linvel();
                    msgData.puck = [pTrans.x, pTrans.y, pTrans.z];
                    msgData.vel = [pVel.x, pVel.y, pVel.z];
                    msgData.score = score;
                }
                invoke("host_send_msg", { msg: JSON.stringify(msgData) }).catch(() => {});
            } else {
                const cTrans = clientPaddleBody ? clientPaddleBody.translation() : {x:0, y:0.2, z:-4};
                let msgData = {
                    type: "input",
                    pos: [cTrans.x, cTrans.y, cTrans.z],
                    puck: undefined as undefined | number[],
                    vel: undefined as undefined | number[],
                    score: undefined as undefined | number[]
                };
                if (amIAuthoritative && puckRigidBody) {
                    const pTrans = puckRigidBody.translation();
                    const pVel = puckRigidBody.linvel();
                    msgData.puck = [pTrans.x, pTrans.y, pTrans.z];
                    msgData.vel = [pVel.x, pVel.y, pVel.z];
                    msgData.score = score;
                }
                invoke("client_send_msg", { msg: JSON.stringify(msgData) }).catch(() => {});
            }
        }
    });

    `;

code = code.substring(0, useTaskStart) + replacement + code.substring(onPointerMoveStart);
fs.writeFileSync('src/lib/components/Scene.svelte', code);
