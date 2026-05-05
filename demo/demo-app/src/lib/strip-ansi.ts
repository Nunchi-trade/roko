/**
 * Canonical ANSI/terminal escape stripping.
 *
 * Single source of truth — all other modules import or re-export from here.
 * Handles CSI, private mode, OSC, charset, single-char escapes, and control chars.
 */
export function stripAnsi(s: string): string {
  return s
    .replace(/\x1b\[\?[0-9;]*[hl]/g, '')     // Private mode set/reset (e.g. ?2004h/l bracketed paste)
    .replace(/\x1b\[[0-9;]*[A-Za-z]/g, '')   // CSI sequences
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '') // OSC sequences
    .replace(/\x1b[()][A-B0-2]/g, '')         // Charset selection
    .replace(/\x1b[DEHMN78]/g, '')            // Single-char escapes
    .replace(/[\x00-\x08\x0e-\x1f]/g, '');   // Control chars (except \t \n \r)
}
