const focusableSelector = [
  "button:not([disabled])",
  "a[href]",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "summary",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

export function focusDialogStart(dialog: HTMLDialogElement): void {
  const preferred = dialog.querySelector<HTMLElement>("[data-dialog-initial-focus]");
  const first = preferred ?? focusableElements(dialog)[0];
  (first ?? dialog).focus({ preventScroll: true });
}

export function trapDialogFocus(event: KeyboardEvent, dialog: HTMLDialogElement | null): void {
  if (event.key !== "Tab" || !dialog?.open) return;

  const focusable = focusableElements(dialog);
  if (focusable.length === 0) {
    event.preventDefault();
    dialog.focus({ preventScroll: true });
    return;
  }

  const first = focusable[0];
  const last = focusable.at(-1) ?? first;
  const active = document.activeElement;

  if (event.shiftKey && (active === first || !dialog.contains(active))) {
    event.preventDefault();
    last.focus({ preventScroll: true });
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus({ preventScroll: true });
  }
}

function focusableElements(dialog: HTMLDialogElement): HTMLElement[] {
  return [...dialog.querySelectorAll<HTMLElement>(focusableSelector)].filter((element) => {
    const closedDisclosure = element.closest<HTMLDetailsElement>("details:not([open])");
    const hiddenByDisclosure =
      closedDisclosure !== null && element !== closedDisclosure.querySelector("summary");
    return (
      !element.hidden &&
      !hiddenByDisclosure &&
      element.getAttribute("aria-hidden") !== "true" &&
      element.getClientRects().length > 0
    );
  });
}
