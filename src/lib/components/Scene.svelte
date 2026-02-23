<script lang="ts">
    import { T, useTask, useThrelte } from "@threlte/core";
    import { RigidBody, Collider, AutoColliders } from "@threlte/rapier";
    import { interactivity } from "@threlte/extras";
    import { onMount, onDestroy } from "svelte";
    import type { RigidBody as RapierRigidBody } from "@dimforge/rapier3d-compat";

    import { invoke } from "@tauri-apps/api/core";
    import { listen, type UnlistenFn } from "@tauri-apps/api/event";
    import { playHit, playWall, playGoal } from "$lib/audio";

    let {
        wsConnection,
        isHost,
        isSinglePlayer = false,
    } = $props<{
        wsConnection: any; // unused now
        isHost: boolean;
        isSinglePlayer?: boolean;
    }>();

    interactivity();

    let puckPos = $state([0, 0.5, 0] as [number, number, number]);
    let hostPaddlePos = $state([0, 0.2, 4] as [number, number, number]);
    let clientPaddlePos = $state([0, 0.2, -4] as [number, number, number]);

    const { size } = useThrelte();
    let cameraY = $derived.by(() => {
        const aspect = $size.width / $size.height;
        if (aspect < 0.6) return 14; // Very narrow mobile
        if (aspect < 1) return 11; // Normal portrait
        return 8; // Desktop / Landscape
    });

    let score = $state([0, 0] as [number, number]);

    let puckRigidBody = $state<RapierRigidBody | undefined>();
    let hostPaddleBody = $state<RapierRigidBody | undefined>();
    let clientPaddleBody = $state<RapierRigidBody | undefined>();

    // Track the pointer intersection with the game board
    let pointerTarget = $state({ x: 0, z: 0 });

    // Interpolation Targets for Client
    let targetPuckPos = { x: 0, y: 0.1, z: 0 };
    let targetHostPaddlePos = { x: 0, y: 0.2, z: 4 };

    // Interpolation Targets for Host (receiving client moves)
    let targetClientPaddlePos = { x: 0, y: 0.2, z: -4 };
    let targetPuckVel = { x: 0, y: 0, z: 0 };

    let puckZ = $derived(puckPos[2] ?? 0);
    const INTERP_SPEED = 0.35;
    let amIAuthoritative = $derived(
        isSinglePlayer ? true : isHost ? puckZ >= 0 : puckZ < 0,
    );
    let wasAuthoritative = true;
    let goalPulse = $state(0);
    let wallPulse = $state(0);
    let puckSpeed = $state(0);
    let trailPositions = $state<[number, number, number][]>([]);
    let goalFlash = $state(false);
    let scoreFlash = $state([false, false] as [boolean, boolean]);

    let wallEmissive = $derived(Math.max(goalPulse * 0.6, wallPulse));

    $effect(() => {
        if (puckRigidBody && amIAuthoritative !== wasAuthoritative) {
            wasAuthoritative = amIAuthoritative;
            if (amIAuthoritative) {
                puckRigidBody.setLinvel(targetPuckVel, true);
            }
        }
    });

    $effect(() => {
        if (goalPulse > 0) {
            const timer = setTimeout(() => {
                goalPulse = Math.max(0, goalPulse - 0.1);
            }, 16);
            return () => clearTimeout(timer);
        }
    });

    $effect(() => {
        if (wallPulse > 0) {
            const timer = setTimeout(() => {
                wallPulse = Math.max(0, wallPulse - 0.18);
            }, 16);
            return () => clearTimeout(timer);
        }
    });

    let unlisten: UnlistenFn | null = null;
    onMount(async () => {
        pointerTarget = { x: 0, z: isHost ? 4 : -4 };
        unlisten = await listen<[string, string]>(
            "udp-msg-received",
            (event) => {
                try {
                    const msg = JSON.parse(event.payload[1]);
                    if (isHost) {
                        if (msg.type === "input") {
                            // Paddle pos is [x, z]
                            targetClientPaddlePos = {
                                x: msg.pos[0],
                                y: 0.2,
                                z: msg.pos[1],
                            };
                            // Authoritative client sends puck/vel/score
                            if (msg.puck) {
                                targetPuckPos = {
                                    x: msg.puck[0],
                                    y: 0.1,
                                    z: msg.puck[1],
                                };
                                if (msg.vel)
                                    targetPuckVel = {
                                        x: msg.vel[0],
                                        y: 0,
                                        z: msg.vel[1],
                                    };
                                if (msg.score) score = msg.score;

                                if (puckRigidBody && !amIAuthoritative) {
                                    const curPuck = puckRigidBody.translation();
                                    if (Math.abs(curPuck.z - msg.puck[1]) > 2) {
                                        puckRigidBody.setTranslation(
                                            targetPuckPos,
                                            true,
                                        );
                                    }
                                }
                            }
                        }
                    } else {
                        if (msg.type === "state") {
                            if (msg.score) score = msg.score;
                            // Host paddle pos is [x, z]
                            if (msg.hostPaddle) {
                                targetHostPaddlePos = {
                                    x: msg.hostPaddle[0],
                                    y: 0.2,
                                    z: msg.hostPaddle[1],
                                };
                            }
                            // Authoritative host sends puck/vel
                            if (msg.puck) {
                                targetPuckPos = {
                                    x: msg.puck[0],
                                    y: 0.1,
                                    z: msg.puck[1],
                                };
                                if (msg.vel)
                                    targetPuckVel = {
                                        x: msg.vel[0],
                                        y: 0,
                                        z: msg.vel[1],
                                    };

                                if (puckRigidBody && !amIAuthoritative) {
                                    const curPuck = puckRigidBody.translation();
                                    if (Math.abs(curPuck.z - msg.puck[1]) > 2) {
                                        puckRigidBody.setTranslation(
                                            targetPuckPos,
                                            true,
                                        );
                                    }
                                }
                            }
                        }
                    }
                } catch (e) {}
            },
        );
    });

    // Track puck speed to detect collision impulses
    let prevPuckSpeed = 0;
    // ────────────────────────────────────────────────────────────────────────

    onDestroy(() => {
        if (unlisten) unlisten();
    });

    useTask(() => {
        if (isHost) {
            // Local Host Paddle
            if (hostPaddleBody) {
                hostPaddleBody.setNextKinematicTranslation({
                    x: pointerTarget.x,
                    y: 0.2,
                    z: pointerTarget.z,
                });
                const t = hostPaddleBody.translation();
                hostPaddlePos = [t.x, 0.2, t.z];
            }

            // Remote Client Paddle (Multiplayer only)
            if (!isSinglePlayer && clientPaddleBody) {
                const t = clientPaddleBody.translation();
                clientPaddleBody.setNextKinematicTranslation({
                    x: t.x + (targetClientPaddlePos.x - t.x) * 0.4,
                    y: 0.2,
                    z: t.z + (targetClientPaddlePos.z - t.z) * 0.4,
                });
                clientPaddlePos = [t.x, 0.2, t.z];
            } else if (isSinglePlayer && clientPaddleBody) {
                // Single Player AI Logic
                const t = clientPaddleBody.translation();
                const pPos = puckRigidBody
                    ? puckRigidBody.translation()
                    : { x: 0, y: 0.1, z: 0 };
                const pVel = puckRigidBody
                    ? puckRigidBody.linvel()
                    : { x: 0, y: 0, z: 0 };

                let aiSpeedMultiplier = 0.03;
                if (pPos.z < 0 || pVel.z < 0) aiSpeedMultiplier = 0.08;

                let nextX = t.x + (pPos.x - t.x) * aiSpeedMultiplier;
                nextX = Math.max(-2.5, Math.min(2.5, nextX));

                let targetZ = Math.min(-0.5, pPos.z);
                if (pPos.z > 0) targetZ = -4.0;

                let nextZ = t.z + (targetZ - t.z) * (aiSpeedMultiplier * 1.5);
                nextZ = Math.min(-0.5, Math.max(-4.5, nextZ));

                const speedMult = 20.0;
                const vx = (nextX - t.x) * speedMult;
                const vz = (nextZ - t.z) * speedMult;
                clientPaddleBody.setNextKinematicTranslation({
                    x: nextX,
                    y: 0.2,
                    z: nextZ,
                });
                clientPaddlePos = [nextX, 0.2, nextZ];
            }
        } else {
            // CLIENT LOGIC
            // Local Client Paddle
            if (clientPaddleBody) {
                clientPaddleBody.setNextKinematicTranslation({
                    x: pointerTarget.x,
                    y: 0.2,
                    z: pointerTarget.z,
                });
                const t = clientPaddleBody.translation();
                clientPaddlePos = [t.x, 0.2, t.z];
            }

            // Remote Host Paddle
            if (hostPaddleBody) {
                hostPaddleBody.setNextKinematicTranslation({
                    x: targetHostPaddlePos.x,
                    y: 0.2,
                    z: targetHostPaddlePos.z,
                });
                const t = hostPaddleBody.translation();
                hostPaddlePos = [t.x, 0.2, t.z];
            }
        }

        // --- PUCK PHYSICS (Authoritative only) ---
        if (puckRigidBody) {
            const pTrans = puckRigidBody.translation();

            if (amIAuthoritative) {
                const hTrans = hostPaddleBody
                    ? hostPaddleBody.translation()
                    : { x: 0, z: 4 };
                const cTrans = clientPaddleBody
                    ? clientPaddleBody.translation()
                    : { x: 0, z: -4 };

                // Sound
                const vel = puckRigidBody.linvel();
                const speed = Math.sqrt(vel.x * vel.x + vel.z * vel.z);
                if (speed - prevPuckSpeed > 1.5) {
                    const dH = Math.hypot(
                        pTrans.x - hTrans.x,
                        pTrans.z - hTrans.z,
                    );
                    const dC = Math.hypot(
                        pTrans.x - cTrans.x,
                        pTrans.z - cTrans.z,
                    );
                    if (dH < 1.1 || dC < 1.1) playHit();
                    else playWall();
                }
                prevPuckSpeed = speed;

                // Stability
                const av = puckRigidBody.angvel();
                puckRigidBody.setAngvel({ x: 0, y: av.y, z: 0 }, true);

                let py = pTrans.y;
                let vy = vel.y;
                if (Math.abs(py - 0.1) > 0.04) {
                    py = 0.1;
                    vy = 0;
                }

                // Boundaries
                const xMax = 2.9; // 3.2 (inner wall) - 0.3 (puck radius)
                let px = pTrans.x,
                    vx = vel.x;
                if (px < -xMax) {
                    px = -xMax;
                    if (vx < 0) {
                        vx = -vx * 0.85;
                        playWall();
                        wallPulse = 1.0;
                    }
                }
                if (px > xMax) {
                    px = xMax;
                    if (vx > 0) {
                        vx = -vx * 0.85;
                        playWall();
                        wallPulse = 1.0;
                    }
                }

                const zMax = 4.9; // 5.2 (inner wall) - 0.3 (puck radius)
                let pz = pTrans.z,
                    vz = vel.z;
                const inGoalGap = Math.abs(px) < 1.3;
                if (!inGoalGap) {
                    if (pz < -zMax) {
                        pz = -zMax;
                        if (vz < 0) {
                            vz = -vz * 0.85;
                            playWall();
                            wallPulse = 1.0;
                        }
                    }
                    if (pz > zMax) {
                        pz = zMax;
                        if (vz > 0) {
                            vz = -vz * 0.85;
                            playWall();
                            wallPulse = 1.0;
                        }
                    }
                }

                const moved =
                    Math.abs(px - pTrans.x) > 0.001 ||
                    Math.abs(pz - pTrans.z) > 0.001 ||
                    Math.abs(py - pTrans.y) > 0.001;
                if (moved) {
                    puckRigidBody.setTranslation({ x: px, y: py, z: pz }, true);
                    puckRigidBody.setLinvel({ x: vx, y: vy, z: vz }, true);
                }

                // Goals
                let goalScored = false;
                if (pTrans.z > 5.4) {
                    score[1]++;
                    playGoal();
                    goalPulse = 1.0;
                    goalFlash = true;
                    scoreFlash = [scoreFlash[0], true];
                    setTimeout(() => {
                        goalFlash = false;
                    }, 500);
                    setTimeout(() => {
                        scoreFlash = [scoreFlash[0], false];
                    }, 700);
                    puckRigidBody.setTranslation({ x: 0, y: 0.1, z: 2.5 }, true);
                    puckRigidBody.setLinvel({ x: 0, y: 0, z: 0 }, true);
                    puckPos = [0, 0.1, 2.5];
                    goalScored = true;
                } else if (pTrans.z < -5.4) {
                    score[0]++;
                    playGoal();
                    goalPulse = 1.0;
                    goalFlash = true;
                    scoreFlash = [true, scoreFlash[1]];
                    setTimeout(() => {
                        goalFlash = false;
                    }, 500);
                    setTimeout(() => {
                        scoreFlash = [false, scoreFlash[1]];
                    }, 700);
                    puckRigidBody.setTranslation({ x: 0, y: 0.1, z: -2.5 }, true);
                    puckRigidBody.setLinvel({ x: 0, y: 0, z: 0 }, true);
                    puckPos = [0, 0.1, -2.5];
                    goalScored = true;
                }

                // Speed limits — keep the game lively
                if (!goalScored) {
                const curVel = puckRigidBody.linvel();
                const curSpeed = Math.sqrt(curVel.x ** 2 + curVel.z ** 2);
                if (curSpeed > 0.3 && curSpeed < 3) {
                    const s = 3 / curSpeed;
                    puckRigidBody.setLinvel(
                        { x: curVel.x * s, y: 0, z: curVel.z * s },
                        true,
                    );
                } else if (curSpeed > 20) {
                    const s = 20 / curSpeed;
                    puckRigidBody.setLinvel(
                        { x: curVel.x * s, y: 0, z: curVel.z * s },
                        true,
                    );
                }
                }

                if (!goalScored) puckPos = [pTrans.x, 0.1, pTrans.z];
            } else {
                // Non-authoritative Interpolation
                const t = puckRigidBody.translation();
                puckRigidBody.setTranslation(
                    {
                        x: t.x + (targetPuckPos.x - t.x) * INTERP_SPEED,
                        y: 0.1,
                        z: t.z + (targetPuckPos.z - t.z) * INTERP_SPEED,
                    },
                    true,
                );
                puckPos = [t.x, 0.1, t.z];
            }
        }

        // Trail + speed
        if (puckRigidBody) {
            const v = puckRigidBody.linvel();
            puckSpeed = Math.sqrt(v.x ** 2 + v.z ** 2);
        }
        trailPositions.unshift([puckPos[0], 0.05, puckPos[2]]);
        if (trailPositions.length > 5) trailPositions.pop();

        // --- NETWORK BROADCAST ---
        if (!isSinglePlayer) {
            if (isHost) {
                const hT = hostPaddleBody
                    ? hostPaddleBody.translation()
                    : { x: 0, z: 4 };
                let msg: any = { type: "state", hostPaddle: [hT.x, hT.z] };
                if (amIAuthoritative && puckRigidBody) {
                    const pT = puckRigidBody.translation();
                    const pV = puckRigidBody.linvel();
                    msg.puck = [pT.x, pT.z];
                    msg.vel = [pV.x, pV.z];
                    msg.score = score;
                }
                invoke("host_send_msg", { msg: JSON.stringify(msg) }).catch(
                    () => {},
                );
            } else {
                const cT = clientPaddleBody
                    ? clientPaddleBody.translation()
                    : { x: 0, z: -4 };
                let msg: any = { type: "input", pos: [cT.x, cT.z] };
                if (amIAuthoritative && puckRigidBody) {
                    const pT = puckRigidBody.translation();
                    const pV = puckRigidBody.linvel();
                    msg.puck = [pT.x, pT.z];
                    msg.vel = [pV.x, pV.z];
                    msg.score = score;
                }
                invoke("client_send_msg", { msg: JSON.stringify(msg) }).catch(
                    () => {},
                );
            }
        }
    });

    function onPointerMove(e: any) {
        if (e.point) {
            // Restrict movement to own half
            let clampedZ = e.point.z;
            if (isHost) {
                clampedZ = Math.max(0.5, Math.min(4.5, e.point.z));
            } else {
                clampedZ = Math.min(-0.5, Math.max(-4.5, e.point.z));
            }
            // Penetration Prevention: Ensure paddle doesn't force puck out or penetrate it
            const dx = e.point.x - puckPos[0];
            const dz = clampedZ - puckPos[2];
            const dist = Math.sqrt(dx * dx + dz * dz);
            const minDist = 0.35 + 0.3; // Paddle radius + Puck radius

            let finalX = Math.max(-2.5, Math.min(2.5, e.point.x));
            let finalZ = clampedZ;

            if (dist < minDist && dist > 0.01) {
                // If paddle is overlapping puck, push pointerTarget slightly away or clamp it
                // This prevents the "force push" through walls
                const overlap = minDist - dist;
                finalX += (dx / dist) * overlap * 0.5;
                finalZ += (dz / dist) * overlap * 0.5;
            }

            pointerTarget = {
                x: Math.max(-2.8, Math.min(2.8, finalX)),
                z: finalZ,
            };
        }
    }
</script>

<T.PerspectiveCamera
    makeDefault={true as any}
    position={[0, cameraY, isHost ? 7 : -7]}
    oncreate={(ref) => ref.lookAt(0, 0, 0)}
/>

<T.AmbientLight intensity={0.4} />
<T.DirectionalLight position={[5, 10, 5]} intensity={1} castShadow />
<T.PointLight
    position={[0, 5, 0]}
    intensity={1.5}
    color="#60a5fa"
    distance={15}
/>

<!-- Goal flash overlay -->
{#if goalFlash}
    <div
        class="absolute inset-0 pointer-events-none z-20"
        style="background: radial-gradient(ellipse at center, rgba(96,165,250,0.45), transparent 70%); animation: fadeOut 0.5s forwards;"
    ></div>
{/if}

<!-- Score Display -->
<div
    class="absolute top-6 left-0 right-0 pointer-events-none flex justify-center items-start z-10 w-full h-full gap-24"
>
    <div
        class="font-black transition-all duration-150 drop-shadow-lg"
        class:text-emerald-300={!isHost}
        class:text-neutral-600={isHost}
        class:text-6xl={!scoreFlash[1]}
        class:text-8xl={scoreFlash[1]}
        style={scoreFlash[1] ? "filter: drop-shadow(0 0 16px #10b981);" : ""}
    >
        {score[1]}
    </div>
    <div
        class="font-black transition-all duration-150 drop-shadow-lg"
        class:text-blue-300={isHost}
        class:text-neutral-600={!isHost}
        class:text-6xl={!scoreFlash[0]}
        class:text-8xl={scoreFlash[0]}
        style={scoreFlash[0] ? "filter: drop-shadow(0 0 16px #3b82f6);" : ""}
    >
        {score[0]}
    </div>
</div>

<!-- Interaction Plane -->
<T.Mesh
    visible={false}
    position={[0, 0.5, 0]}
    rotation.x={-Math.PI / 2}
    onpointermove={onPointerMove}
    onpointerdown={onPointerMove}
>
    <T.PlaneGeometry args={[20, 20]} />
    <T.MeshBasicMaterial transparent opacity={0} />
</T.Mesh>

<!-- Table -->
<RigidBody type="fixed">
    <T.Group position={[0, -0.1, 0]}>
        <!-- Extend floor collider width (8.0 total) so it perfectly goes UNDER the walls -->
        <Collider shape="cuboid" args={[4.0, 0.1, 5.2]} friction={0} />
        <!-- Floor Visual -->
        <T.Mesh receiveShadow>
            <T.BoxGeometry args={[6.4, 0.2, 10.4]} />
            <T.MeshStandardMaterial
                color="#0f172a"
                roughness={0.1}
                metalness={0.8}
            />
        </T.Mesh>
        <!-- Grid Lines -->
        <T.Mesh position={[0, 0.11, 0]} rotation.x={-Math.PI / 2}>
            <T.PlaneGeometry args={[6.4, 10.4]} />
            <T.MeshBasicMaterial transparent opacity={0.2}>
                <T.CanvasTexture
                    oncreate={(ref) => {
                        const canvas = document.createElement("canvas");
                        canvas.width = 512;
                        canvas.height = 1024;
                        const ctx = canvas.getContext("2d");
                        if (ctx) {
                            ctx.strokeStyle = "#3b82f6";
                            ctx.lineWidth = 2;
                            // Horizontal lines
                            for (let i = 0; i <= 10; i++) {
                                ctx.beginPath();
                                ctx.moveTo(0, i * 102.4);
                                ctx.lineTo(512, i * 102.4);
                                ctx.stroke();
                            }
                            // Vertical lines
                            for (let i = 0; i <= 6; i++) {
                                ctx.beginPath();
                                ctx.moveTo(i * 85.3, 0);
                                ctx.lineTo(i * 85.3, 1024);
                                ctx.stroke();
                            }
                            // Center line
                            ctx.strokeStyle = "#f43f5e";
                            ctx.lineWidth = 8;
                            ctx.beginPath();
                            ctx.moveTo(0, 512);
                            ctx.lineTo(512, 512);
                            ctx.stroke();
                        }
                        ref.image = canvas;
                        ref.needsUpdate = true;
                    }}
                />
            </T.MeshBasicMaterial>
        </T.Mesh>
    </T.Group>

    <!-- Visual Walls and Colliders -->
    <!-- Left Wall -->
    <T.Group position={[-3.3, 0.2, 0]}>
        <Collider
            shape="cuboid"
            args={[0.1, 0.3, 5.2]}
            restitution={0.9}
            friction={0}
        />
        <T.Mesh castShadow receiveShadow>
            <T.BoxGeometry args={[0.2, 0.4, 10.4]} />
            <T.MeshStandardMaterial
                color="#94a3b8"
                emissive="#3b82f6"
                emissiveIntensity={wallEmissive}
            />
        </T.Mesh>
    </T.Group>

    <!-- Right Wall -->
    <T.Group position={[3.3, 0.2, 0]}>
        <Collider
            shape="cuboid"
            args={[0.1, 0.3, 5.2]}
            restitution={0.9}
            friction={0}
        />
        <T.Mesh castShadow receiveShadow>
            <T.BoxGeometry args={[0.2, 0.4, 10.4]} />
            <T.MeshStandardMaterial
                color="#94a3b8"
                emissive="#3b82f6"
                emissiveIntensity={wallEmissive}
            />
        </T.Mesh>
    </T.Group>

    <!-- Top Wall Left (Client side) -->
    <T.Group position={[-2.2, 0.2, -5.3]}>
        <Collider
            shape="cuboid"
            args={[1.0, 0.3, 0.1]}
            restitution={0.9}
            friction={0}
        />
        <T.Mesh castShadow receiveShadow>
            <T.BoxGeometry args={[2.0, 0.4, 0.2]} />
            <T.MeshStandardMaterial
                color="#94a3b8"
                emissive="#3b82f6"
                emissiveIntensity={wallEmissive}
            />
        </T.Mesh>
    </T.Group>

    <!-- Top Wall Right (Client side) -->
    <T.Group position={[2.2, 0.2, -5.3]}>
        <Collider
            shape="cuboid"
            args={[1.0, 0.3, 0.1]}
            restitution={0.9}
            friction={0}
        />
        <T.Mesh castShadow receiveShadow>
            <T.BoxGeometry args={[2.0, 0.4, 0.2]} />
            <T.MeshStandardMaterial
                color="#94a3b8"
                emissive="#3b82f6"
                emissiveIntensity={wallEmissive}
            />
        </T.Mesh>
    </T.Group>

    <!-- Bottom Wall Left (Host side) -->
    <T.Group position={[-2.2, 0.2, 5.3]}>
        <Collider
            shape="cuboid"
            args={[1.0, 0.3, 0.1]}
            restitution={0.9}
            friction={0}
        />
        <T.Mesh castShadow receiveShadow>
            <T.BoxGeometry args={[2.0, 0.4, 0.2]} />
            <T.MeshStandardMaterial
                color="#94a3b8"
                emissive="#3b82f6"
                emissiveIntensity={wallEmissive}
            />
        </T.Mesh>
    </T.Group>

    <!-- Bottom Wall Right (Host side) -->
    <T.Group position={[2.2, 0.2, 5.3]}>
        <Collider
            shape="cuboid"
            args={[1.0, 0.3, 0.1]}
            restitution={0.9}
            friction={0}
        />
        <T.Mesh castShadow receiveShadow>
            <T.BoxGeometry args={[2.0, 0.4, 0.2]} />
            <T.MeshStandardMaterial
                color="#94a3b8"
                emissive="#3b82f6"
                emissiveIntensity={wallEmissive}
            />
        </T.Mesh>
    </T.Group>
</RigidBody>

<!-- Corner fillets — rounded cylinders that smoothly deflect the puck off corners -->
{#each [[-3.2, 5.2], [3.2, 5.2], [-3.2, -5.2], [3.2, -5.2]] as [cx, cz]}
    <RigidBody type="fixed">
        <T.Group position={[cx, 0.22, cz]}>
            <Collider shape="cylinder" args={[0.22, 0.35]} restitution={0.7} />
            <T.Mesh castShadow>
                <T.CylinderGeometry args={[0.35, 0.35, 0.44, 24]} />
                <T.MeshStandardMaterial color="#64748b" roughness={0.3} />
            </T.Mesh>
        </T.Group>
    </RigidBody>
{/each}

<!-- PUCK -->
<T.Group position={puckPos}>
    <!-- Subtle Glow Shadow under puck -->
    <T.Mesh position={[0, -0.09, 0]} rotation.x={-Math.PI / 2}>
        <T.PlaneGeometry args={[0.9, 0.9]} />
        <T.MeshBasicMaterial color="#fef08a" transparent opacity={0.15}>
            <T.CanvasTexture
                oncreate={(ref) => {
                    const canvas = document.createElement("canvas");
                    canvas.width = 64;
                    canvas.height = 64;
                    const ctx = canvas.getContext("2d");
                    if (ctx) {
                        const grad = ctx.createRadialGradient(
                            32,
                            32,
                            0,
                            32,
                            32,
                            32,
                        );
                        grad.addColorStop(0, "rgba(254, 240, 138, 1)");
                        grad.addColorStop(1, "rgba(254, 240, 138, 0)");
                        ctx.fillStyle = grad;
                        ctx.fillRect(0, 0, 64, 64);
                    }
                    ref.image = canvas;
                    ref.needsUpdate = true;
                }}
            />
        </T.MeshBasicMaterial>
    </T.Mesh>

    <RigidBody
        bind:rigidBody={puckRigidBody}
        type={amIAuthoritative ? "dynamic" : "kinematicPosition"}
        linearDamping={0.02}
        angularDamping={5}
        canSleep={false}
        ccd={true}
        lockRotations={false}
        enabledRotations={[false, true, false]}
        enabledTranslations={[true, false, true]}
    >
        <!-- Svelte 5 syntax for Threlte Collider child requires wrapping in a group or direct -->
        <Collider
            shape="cylinder"
            args={[0.1, 0.3]}
            restitution={0.85}
            friction={0}
        />
        <T.Mesh castShadow>
            <T.CylinderGeometry args={[0.3, 0.3, 0.2, 32]} />
            <T.MeshStandardMaterial
                color="#fef08a"
                emissive="#ca8a04"
                emissiveIntensity={0.5}
            />
        </T.Mesh>
    </RigidBody>
</T.Group>

<!-- HOST PADDLE -->
<T.Group position={hostPaddlePos}>
    <RigidBody
        bind:rigidBody={hostPaddleBody}
        type="kinematicPosition"
        ccd={true}
        lockRotations={true}
    >
        <Collider
            shape="cylinder"
            args={[0.2, 0.35]}
            restitution={0.82}
            friction={0}
        />
        <T.Mesh castShadow>
            <T.CylinderGeometry args={[0.35, 0.35, 0.4, 32]} />
            <T.MeshStandardMaterial
                color="#3b82f6"
                emissive="#3b82f6"
                emissiveIntensity={0.5}
                metalness={0.8}
                roughness={0.2}
            />
        </T.Mesh>
    </RigidBody>
</T.Group>

<!-- CLIENT PADDLE -->
<T.Group position={clientPaddlePos}>
    <RigidBody
        bind:rigidBody={clientPaddleBody}
        type="kinematicPosition"
        ccd={true}
        lockRotations={true}
    >
        <Collider
            shape="cylinder"
            args={[0.2, 0.35]}
            restitution={0.82}
            friction={0}
        />
        <T.Mesh castShadow>
            <T.CylinderGeometry args={[0.35, 0.35, 0.4, 32]} />
            <T.MeshStandardMaterial
                color="#10b981"
                emissive="#10b981"
                emissiveIntensity={0.5}
                metalness={0.8}
                roughness={0.2}
            />
        </T.Mesh>
    </RigidBody>
</T.Group>
