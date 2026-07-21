<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import Game from "$lib/components/Game.svelte";
  import { initAudio } from "$lib/audio";

  const SERVER_ADDR = "13.232.227.123:9876";

  let screen = $state<"menu" | "game" | "online_host" | "online_join">("menu");
  let isHost = $state(false);
  let isSinglePlayer = $state(false);

  let roomCode = $state("");
  let joinCode = $state("");
  let connecting = $state(false);
  let error = $state("");

  async function startOnlineHost() {
    initAudio();
    isHost = true;
    isSinglePlayer = false;
    connecting = true;
    error = "";
    screen = "online_host";
    try {
      const code = await invoke<string>("create_room", { serverAddr: SERVER_ADDR });
      roomCode = code;
      connecting = false;
      // Wait for opponent to join before transitioning to game
      await invoke("wait_for_opponent");
      screen = "game";
    } catch (e: unknown) {
      error = String(e);
      connecting = false;
    }
  }

  function startOnlineJoin() {
    initAudio();
    isHost = false;
    isSinglePlayer = false;
    error = "";
    joinCode = "";
    screen = "online_join";
  }

  async function joinOnlineRoom() {
    if (!joinCode.trim()) return;
    connecting = true;
    error = "";
    try {
      await invoke("join_room", { serverAddr: SERVER_ADDR, roomCode: joinCode.trim() });
      screen = "game";
    } catch (e: unknown) {
      error = String(e);
      connecting = false;
    }
  }

  async function startSinglePlayer() {
    initAudio();
    isHost = true;
    isSinglePlayer = true;
    try {
      await invoke("create_solo", { serverAddr: SERVER_ADDR });
      screen = "game";
    } catch (e: unknown) {
      error = String(e);
    }
  }

  async function cancelSession() {
    roomCode = "";
    joinCode = "";
    connecting = false;
    error = "";
  }
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
          class="w-full py-4 bg-gradient-to-r from-orange-600 to-orange-500 text-white rounded-2xl text-lg font-bold hover:from-orange-500 hover:to-orange-400 active:scale-95 shadow-[0_0_32px_rgba(234,88,12,0.5)] transition-all uppercase tracking-widest border border-orange-400/30"
          onclick={startOnlineHost}
        >🌍 Host Game</button>
        <button
          class="w-full py-4 bg-gradient-to-r from-yellow-600 to-yellow-500 text-white rounded-2xl text-lg font-bold hover:from-yellow-500 hover:to-yellow-400 active:scale-95 shadow-[0_0_32px_rgba(202,138,4,0.5)] transition-all uppercase tracking-widest border border-yellow-400/30"
          onclick={startOnlineJoin}
        >🔑 Join Game</button>
      </div>
      
      <div class="w-full border-t border-neutral-700/50 my-1"></div>
      
      <button
        class="w-full py-4 bg-gradient-to-r from-purple-600 to-purple-500 text-white rounded-2xl text-lg font-bold hover:from-purple-500 hover:to-purple-400 active:scale-95 shadow-[0_0_32px_rgba(147,51,234,0.5)] transition-all uppercase tracking-widest border border-purple-400/30"
        onclick={startSinglePlayer}
      >🤖 vs AI</button>
    </div>

  {:else if screen === "online_host"}
    <div class="flex flex-col gap-6 items-center text-center p-8 w-full max-w-sm">
      {#if connecting}
        <div class="text-5xl animate-spin">🌍</div>
        <h2 class="text-3xl font-black text-orange-400 drop-shadow-lg">Host Game</h2>
        <div class="flex items-center gap-2 text-neutral-300 text-sm font-medium">
          <span class="w-2 h-2 bg-orange-400 rounded-full animate-ping"></span>
          Registering with server…
        </div>
      {:else if error}
        <div class="text-5xl">❌</div>
        <h2 class="text-3xl font-black text-red-400 drop-shadow-lg">Connection Failed</h2>
        <div class="bg-red-500/10 border border-red-500/30 rounded-xl px-4 py-3">
          <p class="text-red-400 text-sm font-medium">{error}</p>
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
        <p class="text-neutral-500 text-xs">Game starts automatically when someone joins</p>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700/50 text-white rounded-xl hover:bg-neutral-600/50 backdrop-blur-sm font-medium transition-all"
        onclick={async () => { invoke("cancel_wait_for_opponent"); await cancelSession(); screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "online_join"}
    <div class="flex flex-col gap-6 items-center text-center p-8 w-full max-w-sm">
      <div class="text-5xl">🔑</div>
      <h2 class="text-3xl font-black text-yellow-400 drop-shadow-lg">Join Game</h2>
      {#if error}
        <div class="bg-yellow-500/10 border border-yellow-500/30 rounded-xl px-4 py-3">
          <p class="text-yellow-400 text-sm font-medium">{error}</p>
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
        disabled={connecting || joinCode.length < 4}
      >{connecting ? "🔄 Joining…" : "▶️ Join Game"}</button>
      <button
        class="w-full py-3 bg-neutral-700/50 text-white rounded-xl hover:bg-neutral-600/50 backdrop-blur-sm font-medium transition-all"
        onclick={async () => { await cancelSession(); screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "game"}
    <div class="absolute inset-0 w-full h-full">
      <Game {isHost} {isSinglePlayer} roomCode={roomCode} onBack={() => {
        screen = "menu";
        isSinglePlayer = false;
        roomCode = "";
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
