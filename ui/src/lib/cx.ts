/** Joins class names, skipping falsy entries. Later classes do not override earlier ones. */
export function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ')
}
