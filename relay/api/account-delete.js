import { confirmDelete, startDelete } from "../lib/portal.js";

export const GET = (request) => confirmDelete(request);
export const POST = (request) => startDelete(request);
