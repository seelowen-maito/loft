/** What the chat says while an agent's messages load, so an empty chat isn't mistaken for a new one. */
const LINES = [
  (name: string) => `${name} is loading your chat…`,
  (name: string) => `${name} is finding your chats…`,
  (name: string) => `${name} is on a mission to load your chat…`,
  (name: string) => `${name} is lost in the woods looking for your chats…`,
  (name: string) => `${name} is dusting off your messages…`,
  (name: string) => `${name} is rummaging through the archives…`,
];

/** The `turn`th line for `name`, starting at `start` (any integer) and wrapping around. */
export function chatLoadingLine(name: string, start: number, turn = 0): string {
  const n = LINES.length;
  return LINES[(((start + turn) % n) + n) % n](name);
}

export const CHAT_LOADING_LINES = LINES.length;
