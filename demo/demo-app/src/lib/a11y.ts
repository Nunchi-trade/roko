/** Keyboard handler: Enter/Space activates, ArrowUp/Down moves focus. */
export function handleRowKeyDown(
  e: React.KeyboardEvent,
  onClick: () => void,
): void {
  if (e.key === 'Enter' || e.key === ' ') {
    e.preventDefault();
    onClick();
  }
  if (e.key === 'ArrowDown') {
    e.preventDefault();
    const next = (e.currentTarget as HTMLElement).nextElementSibling as HTMLElement | null;
    next?.focus();
  }
  if (e.key === 'ArrowUp') {
    e.preventDefault();
    const prev = (e.currentTarget as HTMLElement).previousElementSibling as HTMLElement | null;
    prev?.focus();
  }
}
