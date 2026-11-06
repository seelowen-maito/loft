/** The Start delivery button's label: while the request runs (it can take several GitHub calls) it says so. */
export function startDeliveryLabel(pending: boolean): string {
  return pending ? "Starting delivery…" : "Start delivery";
}
