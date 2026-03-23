<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import Game from "$lib/components/Game.svelte";
  import { onMount, onDestroy, flushSync } from "svelte";
  import { initAudio } from "$lib/audio";
  import QRCode from 'qrcode';
  import QRScanner from '$lib/components/QRScanner.svelte';
  import { cleanupExpiredRooms, createRoom, deleteRoom, findRoomByCode, generateRoomCode } from "$lib/pocketbase";

  let screen = $state<"menu" | "host" | "join" | "game" | "online_host" | "online_join">("menu");
  let isHost = $state(false);
  let isSinglePlayer = $state(false);
  let useUdp = $state(false);

  // LAN
  let lanQrDataUrl = $state('');
  let lanError = $state('');
  let connectingPeer = $state<string | null>(null);

  // Online state
  let roomCode = $state("");
  let joinCode = $state("");
  let onlineConnecting = $state(false);
  let onlineError = $state("");
  let hostedRoomId = $state<string | null>(null);

  async function createRoomWithRetry(nodeAddrJson: string): Promise<{ code: string; roomId: string }> {
    let lastError: unknown = null;
    for (let i = 0; i < 8; i++) {
      const code = generateRoomCode();
      try {
        const roomId = await createRoom(code, nodeAddrJson);
        return { code, roomId };
      } catch (e) {
        lastError = e;
      }
    }
    throw lastError ?? new Error("Failed to create room");
  }

  async function cancelOnlineSession() {
    const roomId = hostedRoomId;
    hostedRoomId = null;
    roomCode = "";
    onlineConnecting = false;
    if (roomId) {
      await deleteRoom(roomId).catch(() => {});
    }
    await invoke("cancel_online").catch(() => {});
  }

  // ── LAN Host ─────────────────────────────────────────────────────────
  async function startHost() {
    initAudio();
    isHost = true;
    useUdp = true;
    lanQrDataUrl = '';
    lanError = '';
    screen = "host";
    try {
      // grab a local address for QR code and start the UDP listen socket
      const ips = (await invoke("get_local_ips")) as string[];
      const localIp = ips[0] || '';
      lanQrDataUrl = await QRCode.toDataURL(localIp, { width: 240, margin: 1 });
      await invoke("start_udp_host");
      // kick off discovery so joiner can find us without QR
      invoke("start_discovery").catch(() => {});
      // Stay on host screen — peer-connected event fires when client joins
    } catch (e) {
      console.error(e);
      lanError = "Failed to start host: " + e;
    }
  }

  // ── LAN Join ─────────────────────────────────────────────────────────
  async function startJoin() {
    initAudio();
    isHost = false;
    useUdp = true;
    connectingPeer = null;
    screen = "join";
    invoke("start_discovery").catch(() => {});
  }

  async function handleScan(peerIp: string) {
    if (connectingPeer) return;
    connectingPeer = "peer";
    try {
      // direct UDP connect; port 8080 is assumed
      await invoke("connect_udp_client", { hostIp: peerIp });
      screen = "game";
    } catch (e: any) {
      connectingPeer = null;
      alert("Failed to connect: " + e);
    }
  }

  // ── Single Player ─────────────────────────────────────────────────────
  function startSinglePlayer() {
    initAudio();
    isHost = true;
    isSinglePlayer = true;
    useUdp = false;
    screen = "game";
  }

  // ── Online Host ───────────────────────────────────────────────────────
  async function startOnlineHost() {
    initAudio();
    isHost = true;
    isSinglePlayer = false;
    useUdp = false;
    onlineError = "";
    onlineConnecting = true;
    hostedRoomId = null;
    screen = "online_host";
    invoke("stop_discovery").catch(() => {});
    try {
      await invoke("start_accept_loop");
      const nodeAddrJson = (await invoke("get_our_node_addr")) as string;
      await cleanupExpiredRooms().catch(() => {});
      const created = await createRoomWithRetry(nodeAddrJson);
      roomCode = created.code;
      hostedRoomId = created.roomId;
    } catch (e: any) {
      onlineError = e?.toString() ?? "Failed to connect";
      await cancelOnlineSession();
    } finally {
      onlineConnecting = false;
    }
  }

  // ── Online Join ────────────────────────────────────────────────────────
  function startOnlineJoin() {
    initAudio();
    isHost = false;
    isSinglePlayer = false;
    onlineError = "";
    joinCode = "";
    screen = "online_join";
    invoke("stop_discovery").catch(() => {});
  }

  async function joinOnlineRoom() {
    if (!joinCode.trim()) return;
    onlineConnecting = true;
    onlineError = "";
    try {
      const room = await findRoomByCode(joinCode.trim());
      if (!room) {
        throw new Error(`Room '${joinCode.trim()}' not found`);
      }
      await invoke("connect_to_peer", { nodeAddrJson: room.node_addr });
      await deleteRoom(room.id).catch(() => {});
      // peer-connected event fires from connect_to_peer → screen = "game"
    } catch (e: any) {
      onlineError = e?.toString() ?? "Failed to join";
      onlineConnecting = false;
    }
  }

  // ── Event listeners ────────────────────────────────────────────────────
  let unlistenPeerConnected: UnlistenFn | null = null;
  let unlistenPeerFound: UnlistenFn | null = null;

  onMount(async () => {
    unlistenPeerConnected = await listen("peer-connected", () => {
      connectingPeer = null;
      onlineConnecting = false;
      if (hostedRoomId) {
        const roomId = hostedRoomId;
        hostedRoomId = null;
        deleteRoom(roomId).catch(() => {});
      }
      flushSync(() => { screen = "game"; });
    });

    unlistenPeerFound = await listen<string>("peer-found", (event) => {
      const ip = event.payload;
      // auto-join if we're on the LAN join screen
      if (screen === "join" && !connectingPeer) {
        handleScan(ip);
      }
    });

    // always run discovery so peers see us
    invoke("start_discovery").catch(() => {});
  });

  onDestroy(() => {
    if (unlistenPeerConnected) unlistenPeerConnected();
    if (unlistenPeerFound) unlistenPeerFound();
    invoke("stop_discovery").catch(() => {});
    cancelOnlineSession().catch(() => {});
  });
</script>

<main
  class="w-screen h-screen bg-neutral-900 text-white flex flex-col items-center justify-center overflow-hidden"
>
  {#if screen === "menu"}
    <div class="flex flex-col gap-5 items-center px-8 w-full max-w-sm">
      <div class="flex flex-col items-center mb-6">
        <div class="text-5xl mb-3">🏒</div>
        <h1 class="text-4xl font-black tracking-tight bg-gradient-to-r from-cyan-400 to-blue-500 bg-clip-text text-transparent">
          Air Hockey
        </h1>
        <p class="text-neutral-500 text-sm mt-1 tracking-widest uppercase">First to 6 wins</p>
      </div>
      <button
        class="w-full py-4 bg-blue-600 text-white rounded-2xl text-lg font-bold hover:bg-blue-500 active:scale-95 shadow-[0_0_24px_rgba(37,99,235,0.4)] transition-all uppercase tracking-widest"
        onclick={startHost}
      >🌐 Host (Local)</button>
      <button
        class="w-full py-4 bg-emerald-600 text-white rounded-2xl text-lg font-bold hover:bg-emerald-500 active:scale-95 shadow-[0_0_24px_rgba(5,150,105,0.4)] transition-all uppercase tracking-widest"
        onclick={startJoin}
      >📡 Join (Local)</button>
      <div class="w-full border-t border-neutral-700 my-1"></div>
      <button
        class="w-full py-4 bg-orange-600 text-white rounded-2xl text-lg font-bold hover:bg-orange-500 active:scale-95 shadow-[0_0_24px_rgba(234,88,12,0.4)] transition-all uppercase tracking-widest"
        onclick={startOnlineHost}
      >🌍 Host (Online)</button>
      <button
        class="w-full py-4 bg-yellow-600 text-white rounded-2xl text-lg font-bold hover:bg-yellow-500 active:scale-95 shadow-[0_0_24px_rgba(202,138,4,0.4)] transition-all uppercase tracking-widest"
        onclick={startOnlineJoin}
      >🔑 Join (Online)</button>
      <div class="w-full border-t border-neutral-700 my-1"></div>
      <button
        class="w-full py-4 bg-purple-600 text-white rounded-2xl text-lg font-bold hover:bg-purple-500 active:scale-95 shadow-[0_0_24px_rgba(147,51,234,0.4)] transition-all uppercase tracking-widest"
        onclick={startSinglePlayer}
      >🤖 vs AI</button>
      <p class="text-neutral-600 text-xs text-center mt-1">For best experience, use 5 GHz Wi-Fi</p>
    </div>

  {:else if screen === "host"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <h2 class="text-2xl font-black text-cyan-400">Host (Local)</h2>
      {#if lanError}
        <div class="text-red-400 text-sm px-2">{lanError}</div>
        <button
          class="w-full py-3 bg-blue-600 text-white rounded-xl hover:bg-blue-500"
          onclick={startHost}
        >Retry</button>
      {:else if lanQrDataUrl}
        <p class="text-neutral-400 text-sm">Ask your friend to scan this QR code</p>
        <img src={lanQrDataUrl} alt="QR code" class="rounded-2xl w-48 h-48" />
        <p class="text-neutral-500 text-xs animate-pulse">Waiting for opponent…</p>
      {:else}
        <div class="text-3xl animate-pulse">⏳</div>
        <p class="text-neutral-500 text-sm">Generating QR code…</p>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600 mt-2"
        onclick={async () => { await cancelOnlineSession(); useUdp = false; isHost = false; screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "join"}
    <div class="flex flex-col gap-4 items-center w-full p-8 max-w-sm">
      <h2 class="text-2xl font-black text-emerald-400">Join (Local)</h2>
      {#if connectingPeer}
        <div class="text-3xl animate-pulse">🔗</div>
        <p class="text-neutral-400 text-sm">Connecting…</p>
      {:else}
        <p class="text-neutral-500 text-sm text-center">Scan the host's QR code</p>
        <QRScanner onScan={handleScan} />
      {/if}
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600 mt-4"
        onclick={() => { connectingPeer = null; screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "online_host"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <div class="text-3xl animate-pulse">🌍</div>
      <h2 class="text-2xl font-black text-orange-400">Online — Host</h2>
      {#if onlineConnecting}
        <p class="text-neutral-400 text-sm">Registering with signaling server…</p>
      {:else if onlineError}
        <p class="text-red-400 text-sm">{onlineError}</p>
      {:else if roomCode}
        <p class="text-neutral-400 text-sm">Share this code with your friend</p>
        <div class="bg-neutral-800 border border-orange-500/40 rounded-2xl px-10 py-6">
          <p class="text-6xl font-black tracking-widest text-orange-400 font-mono">{roomCode}</p>
        </div>
        <p class="text-neutral-500 text-xs animate-pulse">Waiting for opponent to join…</p>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600 mt-2"
        onclick={async () => { await cancelOnlineSession(); screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "online_join"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <div class="text-3xl">🔑</div>
      <h2 class="text-2xl font-black text-yellow-400">Online — Join</h2>
      <p class="text-neutral-400 text-sm">Enter the 4-digit code from your friend</p>
      <input
        type="text"
        inputmode="numeric"
        pattern="[0-9]*"
        maxlength="4"
        placeholder="0000"
        bind:value={joinCode}
        oninput={(e) => { joinCode = (e.target as HTMLInputElement).value.replace(/\D/g, '').slice(0, 4); }}
        class="w-full px-6 py-5 bg-neutral-800 text-white rounded-2xl border border-neutral-600 focus:border-yellow-500 outline-none font-mono text-4xl font-black tracking-widest text-center"
        onkeydown={(e) => e.key === 'Enter' && joinOnlineRoom()}
      />
      {#if onlineError}
        <p class="text-red-400 text-sm">{onlineError}</p>
      {/if}
      <button
        class="w-full py-4 bg-yellow-600 text-white rounded-2xl text-lg font-bold hover:bg-yellow-500 active:scale-95 disabled:opacity-40 uppercase tracking-widest"
        onclick={joinOnlineRoom}
        disabled={onlineConnecting || joinCode.length < 4}
      >{onlineConnecting ? "Connecting…" : "Join"}</button>
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600"
        onclick={async () => { await cancelOnlineSession(); screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "game"}
    <div class="absolute inset-0 w-full h-full">
      <Game {isHost} {isSinglePlayer} {useUdp} onBack={() => {
        screen = "menu";
        isSinglePlayer = false;
        useUdp = false;
      }} />
    </div>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    font-family: "Inter", system-ui, sans-serif;
    user-select: none;
    -webkit-user-select: none;
    touch-action: none;
  }
</style>
