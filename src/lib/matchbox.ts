// Matchbox signaling client (replaces PocketBase)
// With matchbox, room management is handled by the backend signaling server.
// These functions are stubs to keep the frontend compatible.

export interface RoomRecord {
  id: string;
  node_addr: string;
}

// Generate a random 4-digit room code (used as fallback, but backend generates code)
export function generateRoomCode(): string {
  return Math.floor(Math.random() * 10000).toString().padStart(4, "0");
}

// No-op - matchbox server handles expiration
export async function cleanupExpiredRooms(): Promise<void> {
  // Nothing to do
}

// Create a room record (stub). Returns a dummy room ID.
export async function createRoom(code: string, nodeAddr: string): Promise<string> {
  console.log('[matchbox] createRoom stub called', code, nodeAddr);
  // Room is created on the matchbox server via host_online command.
  // Return a dummy ID that will be ignored.
  return 'dummy-' + code;
}

// Find room by code (stub). Returns a dummy room record.
export async function findRoomByCode(code: string, timeoutMs = 30_000): Promise<RoomRecord | null> {
  console.log('[matchbox] findRoomByCode stub called', code);
  // The backend join_online command will handle room lookup.
  // Return a dummy record; the frontend expects a RoomRecord but won't use node_addr.
  return {
    id: 'dummy-' + code,
    node_addr: ''
  };
}

// Update joiner address (stub)
export async function updateRoomJoinerAddr(roomId: string, joinerAddr: string): Promise<void> {
  console.log('[matchbox] updateRoomJoinerAddr stub called', roomId, joinerAddr);
  // Nothing to do
}

// Delete room (stub)
export async function deleteRoom(roomId: string): Promise<void> {
  console.log('[matchbox] deleteRoom stub called', roomId);
  // Nothing to do
}