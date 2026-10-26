/** Hero welcome lines: [plain lead, italic accent]. One is picked per page load. */
export const GREETINGS: [string, string][] = [
  ["Ready to build", "something great?"],
  ["Welcome back. Let's", "build something great."],
  ["Good to see you. Time to", "make something great."],
  ["The workshop is open.", "Build something great."],
  ["Fresh start, clear head.", "Let's make it great."],
  ["Pull up a chair. Something great", "starts here."],
  ["All systems ready.", "What are we building today?"],
  ["Welcome home. Ready to", "ship something great?"],
  ["Every great thing starts", "with the first commit."],
  ["Let's turn today into", "something worth shipping."],
  // accelerationist
  ["The future won't build itself.", "Let's speed it up."],
  ["Welcome back. Time to", "accelerate."],
  ["Every commit compounds.", "Let's go exponential."],
  ["Build at the speed of thought.", "Ship at the speed of light."],
  ["The frontier moves every day.", "Let's push it further."],
  ["Less waiting, more shipping.", "The future arrives early."],
  ["Your agents are awake.", "Let's build faster than ever."],
  ["Momentum is a choice.", "Choose full throttle."],
  ["Great things, built faster.", "Welcome to the acceleration."],
  ["The curve only bends one way.", "Up and to the right."],
];

const KEY = "loft:last-greeting";

/** Random greeting, never the same one twice in a row (best effort, per browser). */
export function pickGreeting(): [string, string] {
  let last = -1;
  try {
    last = Number(localStorage.getItem(KEY) ?? -1);
  } catch {
    /* storage unavailable */
  }
  let i = Math.floor(Math.random() * GREETINGS.length);
  if (i === last) i = (i + 1 + Math.floor(Math.random() * (GREETINGS.length - 1))) % GREETINGS.length;
  try {
    localStorage.setItem(KEY, String(i));
  } catch {
    /* storage unavailable */
  }
  return GREETINGS[i];
}
