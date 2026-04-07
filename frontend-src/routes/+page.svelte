<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import Game from "$lib/components/Game.svelte";
  import { onMount, onDestroy, flushSync } from "svelte";
  import { initAudio } from "$lib/audio";
  import QRCode from 'qrcode';
  import QRScanner from '$lib/components/QRScanner.svelte';
  import { cleanupExpiredRooms, createRoom, deleteRoom, findRoomByCode, updateRoomJoinerAddr } from "$lib/matchbox";

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
  let retryRoomId = $state<string | null>(null); // room to post joiner addr to on retry
  const ONLINE_P2P_TIMEOUT_MS = 5000;
  const NETWORK_INCOMPATIBLE_MESSAGE = "Network incompatible. Try different network.";
  let onlineAttemptTimer = $state<ReturnType<typeof setTimeout> | null>(null);
  let onlineAttemptId = $state(0);

  function clearOnlineAttemptTimer() {
    if (!onlineAttemptTimer) return;
    clearTimeout(onlineAttemptTimer);
    onlineAttemptTimer = null;
  }

  function failOnlineAttemptWithNetworkMessage() {
    onlineConnecting = false;
    onlineError = NETWORK_INCOMPATIBLE_MESSAGE;
  }

  function invalidateOnlineAttempt() {
    onlineAttemptId += 1;
    clearOnlineAttemptTimer();
  }

  function startOnlineAttemptTimeout() {
    invalidateOnlineAttempt();
    const attemptId = onlineAttemptId;
    onlineAttemptTimer = setTimeout(() => {
      if (attemptId !== onlineAttemptId) return;
      invoke("cancel_online").catch(() => {});
      failOnlineAttemptWithNetworkMessage();
    }, ONLINE_P2P_TIMEOUT_MS);
  }


  async function cancelOnlineSession(clearError = true) {
    invalidateOnlineAttempt();
    const roomId = hostedRoomId ?? retryRoomId;
    hostedRoomId = null;
    roomCode = "";
    onlineConnecting = false;
    retryRoomId = null;
    if (clearError) onlineError = "";
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
    startOnlineAttemptTimeout();
    isHost = true;
    isSinglePlayer = false;
    useUdp = false;
    onlineError = "";
    onlineConnecting = true;
    hostedRoomId = null;
    screen = "online_host";
    
    try {
      await invoke("stop_discovery").catch(() => {});
      console.log('[Online Host] Calling host_online...');
      
      // Use matchbox host_online command (creates WebRTC socket and registers room)
      const code = await invoke("host_online");
      console.log('[Online Host] Response received:', code);
      
      roomCode = String(code);
      hostedRoomId = 'dummy-' + code;
      onlineConnecting = false;
      console.log('[Online Host] State updated, roomCode:', roomCode);
    } catch (e: any) {
      console.error('[Online Host] Error:', e);
      invalidateOnlineAttempt();
      failOnlineAttemptWithNetworkMessage();
      hostedRoomId = null;
      roomCode = "";
    }
  }

  // ── Online Join ────────────────────────────────────────────────────────
  function startOnlineJoin() {
    initAudio();
    invalidateOnlineAttempt();
    isHost = false;
    isSinglePlayer = false;
    onlineError = "";
    joinCode = "";
    screen = "online_join";
    invoke("stop_discovery").catch(() => {});
  }

  async function joinOnlineRoom() {
    if (!joinCode.trim()) return;
    startOnlineAttemptTimeout();
    onlineConnecting = true;
    onlineError = "";
    retryRoomId = null;
    try {
      // Use matchbox join_online command with the room code
      await invoke("join_online", { roomCode: joinCode.trim() });
      // peer-connected event will be emitted when connection established
      // No need to delete room (matchbox handles cleanup)
    } catch (e: any) {
      invalidateOnlineAttempt();
      failOnlineAttemptWithNetworkMessage();
    }
  }


  // ── Event listeners ────────────────────────────────────────────────────
  let unlistenPeerConnected: UnlistenFn | null = null;
  let unlistenPeerFound: UnlistenFn | null = null;
  let unlistenPeerDisconnected: UnlistenFn | null = null;

  onMount(async () => {
    unlistenPeerConnected = await listen("peer-connected", () => {
      invalidateOnlineAttempt();
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

    unlistenPeerDisconnected = await listen("peer-disconnected", async () => {
      if (screen !== "game") return;
      await invoke("reset_transport").catch(() => {});
      if (isHost) {
        // Re-host based on connection type
        if (useUdp) {
          // LAN mode: re-host with UDP
          await startHost();
        } else {
          // Online mode: re-host with WebRTC
          await startOnlineHost();
        }
      } else {
        // Client side: show appropriate rejoin screen
        if (useUdp) {
          lanError = "Connection lost — tap Host/Join to reconnect.";
          screen = "menu";
        } else {
          onlineError = "Connection lost — tap Join (Online) to reconnect.";
          onlineConnecting = false;
          screen = "online_join";
        }
      }
    });

    // always run discovery so peers see us
    invoke("start_discovery").catch(() => {});
  });

  onDestroy(() => {
    invalidateOnlineAttempt();
    if (unlistenPeerConnected) unlistenPeerConnected();
    if (unlistenPeerFound) unlistenPeerFound();
    if (unlistenPeerDisconnected) unlistenPeerDisconnected();
    invoke("stop_discovery").catch(() => {});
    cancelOnlineSession().catch(() => {});
  });
</script>

<main
  class="w-screen h-screen bg-gradient-to-b from-slate-900 via-neutral-900 to-slate-900 text-white flex flex-col items-center justify-center overflow-hidden"
>
  {#if screen === "menu"}
    <div class="flex flex-col gap-6 items-center px-8 w-full max-w-sm">
      <div class="flex flex-col items-center mb-4">
        <div class="text-6xl mb-4 animate-bounce">🏒</div>
        <h1 class="text-5xl font-black tracking-tight bg-gradient-to-r from-cyan-400 via-blue-500 to-purple-500 bg-clip-text text-transparent drop-shadow-lg">
          Air Hockey
        </h1>
        <p class="text-neutral-400 text-xs mt-2 tracking-widest uppercase font-semibold">First to 6 wins</p>
      </div>
      
      <div class="w-full space-y-3">
        <button
          class="w-full py-4 bg-gradient-to-r from-blue-600 to-blue-500 text-white rounded-2xl text-lg font-bold hover:from-blue-500 hover:to-blue-400 active:scale-95 shadow-[0_0_32px_rgba(37,99,235,0.5)] transition-all uppercase tracking-widest border border-blue-400/30"
          onclick={startHost}
        >🌐 Host (Local)</button>
        <button
          class="w-full py-4 bg-gradient-to-r from-emerald-600 to-emerald-500 text-white rounded-2xl text-lg font-bold hover:from-emerald-500 hover:to-emerald-400 active:scale-95 shadow-[0_0_32px_rgba(5,150,105,0.5)] transition-all uppercase tracking-widest border border-emerald-400/30"
          onclick={startJoin}
        >📡 Join (Local)</button>
      </div>
      
      <div class="w-full border-t border-neutral-700/50 my-1"></div>
      
      <div class="w-full space-y-3">
        <button
          class="w-full py-4 bg-gradient-to-r from-orange-600 to-orange-500 text-white rounded-2xl text-lg font-bold hover:from-orange-500 hover:to-orange-400 active:scale-95 shadow-[0_0_32px_rgba(234,88,12,0.5)] transition-all uppercase tracking-widest border border-orange-400/30"
          onclick={startOnlineHost}
        >🌍 Host (Online)</button>
        <button
          class="w-full py-4 bg-gradient-to-r from-yellow-600 to-yellow-500 text-white rounded-2xl text-lg font-bold hover:from-yellow-500 hover:to-yellow-400 active:scale-95 shadow-[0_0_32px_rgba(202,138,4,0.5)] transition-all uppercase tracking-widest border border-yellow-400/30"
          onclick={startOnlineJoin}
        >🔑 Join (Online)</button>
      </div>
      
      <div class="w-full border-t border-neutral-700/50 my-1"></div>
      
      <button
        class="w-full py-4 bg-gradient-to-r from-purple-600 to-purple-500 text-white rounded-2xl text-lg font-bold hover:from-purple-500 hover:to-purple-400 active:scale-95 shadow-[0_0_32px_rgba(147,51,234,0.5)] transition-all uppercase tracking-widest border border-purple-400/30"
        onclick={startSinglePlayer}
      >🤖 vs AI</button>
      
      <p class="text-neutral-600 text-xs text-center mt-2 font-medium">💡 For best experience, use 5 GHz Wi-Fi</p>
    </div>

  {:else if screen === "host"}
    <div class="flex flex-col gap-6 items-center text-center p-8 w-full max-w-sm">
      <div class="text-5xl animate-pulse">🎯</div>
      <h2 class="text-3xl font-black text-cyan-400 drop-shadow-lg">Host (Local)</h2>
      {#if lanError}
        <div class="bg-red-500/10 border border-red-500/30 rounded-xl px-4 py-3">
          <p class="text-red-400 text-sm font-medium">{lanError}</p>
        </div>
        <button
          class="w-full py-3 bg-gradient-to-r from-blue-600 to-blue-500 text-white rounded-xl hover:from-blue-500 hover:to-blue-400 font-semibold shadow-lg"
          onclick={startHost}
        >🔄 Retry</button>
      {:else if lanQrDataUrl}
        <p class="text-neutral-300 text-sm font-medium">Ask your friend to scan this QR code</p>
        <div class="bg-white p-3 rounded-3xl shadow-[0_0_40px_rgba(34,211,238,0.3)] border-4 border-cyan-400/30">
          <img src={lanQrDataUrl} alt="QR code" class="rounded-2xl w-52 h-52" />
        </div>
        <div class="flex items-center gap-2 text-cyan-400 text-sm font-semibold animate-pulse">
          <span class="w-2 h-2 bg-cyan-400 rounded-full animate-ping"></span>
          Waiting for opponent…
        </div>
      {:else}
        <div class="text-4xl animate-spin">⏳</div>
        <p class="text-neutral-400 text-sm">Generating QR code…</p>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700/50 text-white rounded-xl hover:bg-neutral-600/50 backdrop-blur-sm font-medium transition-all"
        onclick={async () => { await cancelOnlineSession(); useUdp = false; isHost = false; screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "join"}
    <div class="flex flex-col gap-5 items-center w-full p-8 max-w-sm">
      <div class="text-5xl">📱</div>
      <h2 class="text-3xl font-black text-emerald-400 drop-shadow-lg">Join (Local)</h2>
      {#if connectingPeer}
        <div class="text-4xl animate-bounce">🔗</div>
        <p class="text-neutral-300 text-sm font-medium">Connecting…</p>
      {:else}
        <p class="text-neutral-400 text-sm text-center font-medium">Scan the host's QR code with your camera</p>
        <div class="w-full rounded-2xl overflow-hidden border-2 border-emerald-500/30 shadow-[0_0_30px_rgba(16,185,129,0.2)]">
          <QRScanner onScan={handleScan} />
        </div>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700/50 text-white rounded-xl hover:bg-neutral-600/50 backdrop-blur-sm font-medium transition-all mt-2"
        onclick={() => { connectingPeer = null; screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "online_host"}
    <div class="flex flex-col gap-6 items-center text-center p-8 w-full max-w-sm">
      {#if onlineConnecting}
        <div class="text-5xl animate-spin">🌍</div>
        <h2 class="text-3xl font-black text-orange-400 drop-shadow-lg">Online — Host</h2>
        <div class="flex items-center gap-2 text-neutral-300 text-sm font-medium">
          <span class="w-2 h-2 bg-orange-400 rounded-full animate-ping"></span>
          Registering with server…
        </div>
      {:else if onlineError}
        <div class="text-5xl">❌</div>
        <h2 class="text-3xl font-black text-red-400 drop-shadow-lg">Connection Failed</h2>
        <div class="bg-red-500/10 border border-red-500/30 rounded-xl px-4 py-3">
          <p class="text-red-400 text-sm font-medium">{onlineError}</p>
        </div>
        <button
          class="w-full py-3 bg-gradient-to-r from-orange-600 to-orange-500 text-white rounded-xl hover:from-orange-500 hover:to-orange-400 font-semibold shadow-lg"
          onclick={startOnlineHost}
        >🔄 Try Again</button>
      {:else if roomCode}
        <div class="text-5xl animate-bounce">🎉</div>
        <h2 class="text-3xl font-black text-orange-400 drop-shadow-lg">Room Ready!</h2>
        <p class="text-neutral-300 text-sm font-medium">Share this code with your friend</p>
        <div class="bg-gradient-to-br from-orange-500/20 to-orange-600/10 border-2 border-orange-500/40 rounded-3xl px-8 py-6 shadow-[0_0_40px_rgba(249,115,22,0.3)]">
          <p class="text-7xl font-black tracking-widest text-orange-400 font-mono drop-shadow-lg">{roomCode}</p>
        </div>
        <div class="flex items-center gap-2 text-orange-400 text-sm font-semibold animate-pulse">
          <span class="w-2 h-2 bg-orange-400 rounded-full animate-ping"></span>
          Waiting for opponent to join…
        </div>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700/50 text-white rounded-xl hover:bg-neutral-600/50 backdrop-blur-sm font-medium transition-all"
        onclick={async () => { await cancelOnlineSession(); screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "online_join"}
    <div class="flex flex-col gap-6 items-center text-center p-8 w-full max-w-sm">
      <div class="text-5xl">🔑</div>
      <h2 class="text-3xl font-black text-yellow-400 drop-shadow-lg">Join (Online)</h2>
      {#if onlineError}
        <div class="bg-yellow-500/10 border border-yellow-500/30 rounded-xl px-4 py-3">
          <p class="text-yellow-400 text-sm font-medium">{onlineError}</p>
        </div>
      {:else}
        <p class="text-neutral-400 text-sm font-medium">Enter the 4-digit code from your friend</p>
      {/if}
      <input
        type="text"
        inputmode="numeric"
        pattern="[0-9]*"
        maxlength="4"
        placeholder="0000"
        bind:value={joinCode}
        oninput={(e) => { joinCode = (e.target as HTMLInputElement).value.replace(/\D/g, '').slice(0, 4); }}
        class="w-full px-6 py-5 bg-neutral-800/80 text-white rounded-2xl border-2 border-neutral-600 focus:border-yellow-500 outline-none font-mono text-5xl font-black tracking-widest text-center shadow-lg transition-all"
        onkeydown={(e) => e.key === 'Enter' && joinOnlineRoom()}
      />
      <button
        class="w-full py-4 bg-gradient-to-r from-yellow-600 to-yellow-500 text-white rounded-2xl text-lg font-bold hover:from-yellow-500 hover:to-yellow-400 active:scale-95 disabled:opacity-40 disabled:active:scale-100 uppercase tracking-widest shadow-[0_0_24px_rgba(202,138,4,0.4)] transition-all border border-yellow-400/30"
        onclick={joinOnlineRoom}
        disabled={onlineConnecting || joinCode.length < 4}
      >{onlineConnecting ? "🔄 Connecting…" : "▶️ Join Game"}</button>
      <button
        class="w-full py-3 bg-neutral-700/50 text-white rounded-xl hover:bg-neutral-600/50 backdrop-blur-sm font-medium transition-all"
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
