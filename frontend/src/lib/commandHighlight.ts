/** How many leading characters of the composer's `text` to colour as a command: the whole `/name` once it
 * exactly matches one of `names` (in any case), whether or not arguments follow, else 0. */
export function commandSpan(text: string, names: string[]): number {
  const word = /^\/\S*/.exec(text)?.[0];
  return word && names.some((n) => n.toLowerCase() === word.toLowerCase()) ? word.length : 0;
}
