/** Svelte action for a <details> that works as a menu: it closes on a press outside it and on Escape,
 * which a bare <details> does not do. */
export function closeOnOutside(node: HTMLDetailsElement) {
  const onPress = (event: Event) => {
    if (node.open && !node.contains(event.target as Node)) node.open = false;
  };
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Escape") node.open = false;
  };
  document.addEventListener("pointerdown", onPress, true);
  document.addEventListener("keydown", onKey, true);
  return {
    destroy() {
      document.removeEventListener("pointerdown", onPress, true);
      document.removeEventListener("keydown", onKey, true);
    },
  };
}
