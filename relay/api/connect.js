import { connect, approve } from "../lib/pairing.js";

export const GET = (request) => connect(request);
export const POST = (request) => approve(request);
