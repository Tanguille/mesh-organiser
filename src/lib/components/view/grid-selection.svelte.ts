import { handleGridItemKeyDown } from "#lib/utils";
import { state as dragState } from "#lib/drag-selected-models.svelte";

interface GridSelectionOptions<T> {
  getItems: () => T[];
  getSelected: () => T[];
  setSelected: (items: T[]) => void;
  getId: (item: T) => number;
  // Fires right before an interaction (click, right-click, checkbox toggle)
  // touches the selection, including the shift-range early-return path, so
  // callers can invalidate derived state (e.g. group-grid's split-view model
  // sub-selection).
  onBeforeSelect?: () => void;
  getScrollContainer: () => HTMLElement | undefined;
  onEndOfList?: () => void;
  onActivate?: (item: T) => boolean;
  onRename?: (item: T) => void;
}

// Selection state shared by the model/group grids. The selection array stays
// owned by the component (model-grid-inner binds it as a prop, group-grid
// reassigns it from pruning/delete paths), so everything goes through accessors.
export function createGridSelection<T>(options: GridSelectionOptions<T>) {
  const {
    getItems,
    getSelected,
    setSelected,
    getId,
    onBeforeSelect,
    getScrollContainer,
    onEndOfList,
    onActivate,
    onRename,
  } = options;

  // A plain Set is fine here: it is rebuilt from scratch whenever the
  // selection changes and never mutated, so SvelteSet's mutation tracking
  // would buy nothing.
  const selectedSet = $derived(new Set(getSelected().map(getId)));

  let preventOnClick = $state.raw(false);
  let anchorId: number | null = null;
  let focusId = $state<number | null>(null);
  let focusWithin = false;
  let scrollTimeout: ReturnType<typeof setTimeout> | undefined;

  function scrollToTarget(target: EventTarget | null) {
    clearTimeout(scrollTimeout);
    if (target instanceof HTMLElement) {
      scrollTimeout = setTimeout(() => {
        target.scrollIntoView({ behavior: "smooth", block: "center" });
      }, 30);
    }
  }

  function handleScroll() {
    const scrollContainer = getScrollContainer();
    if (!scrollContainer) return;

    const { scrollTop, scrollHeight, clientHeight } = scrollContainer;
    if (Math.round(scrollTop + clientHeight + 10) >= scrollHeight) {
      onEndOfList?.();
    }
  }

  // Poll for the end of the list too: content can shrink/grow (or load in)
  // without a scroll event ever firing.
  const interval = setInterval(handleScroll, 1000);

  function destroy() {
    clearInterval(interval);
    clearTimeout(scrollTimeout);
  }

  function onClick(item: T, event: MouseEvent) {
    if (preventOnClick) {
      preventOnClick = false;
      return;
    }

    onBeforeSelect?.();

    const items = getItems();
    const selected = getSelected();

    if (event.shiftKey && (anchorId !== null || selected.length > 0)) {
      let start = items.findIndex(
        (value) => getId(value) === (anchorId ?? getId(selected[0])),
      );
      let end = items.indexOf(item);

      if (end === -1) {
        return;
      }
      if (start === -1) {
        anchorId = getId(item);
        setSelected([item]);
        return;
      }

      if (start > end) {
        [start, end] = [end, start];
      }

      setSelected(items.slice(start, end + 1));
    } else if (event.ctrlKey || event.metaKey) {
      anchorId = getId(item);
      if (selectedSet.has(getId(item))) {
        setSelected(selected.filter((x) => getId(x) !== getId(item)));
      } else {
        setSelected([...selected, item]);
      }
    } else {
      anchorId = getId(item);
      setSelected([item]);

      scrollToTarget(event.target);
    }
  }

  function earlyOnClick(item: T, event: MouseEvent, isSelected: boolean) {
    preventOnClick = false;
    if (!isSelected) {
      onClick(item, event);
      preventOnClick = true;
    }
  }

  function onRightClick(item: T, event: MouseEvent) {
    if (selectedSet.has(getId(item))) {
      return;
    }

    onBeforeSelect?.();
    setSelected([item]);

    scrollToTarget(event.target);
  }

  function onKeyDown(item: T, event: KeyboardEvent) {
    preventOnClick = false;
    const items = getItems();
    if (event.key === "F2" && onRename) {
      event.preventDefault();
      onRename(item);
      return;
    }
    if (event.key === "Enter" && onActivate?.(item)) {
      event.preventDefault();
      return;
    }
    if (event.key === " ") {
      event.preventDefault();
      onClick(
        item,
        new MouseEvent("click", { ctrlKey: true, shiftKey: event.shiftKey }),
      );
      return;
    }
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
      event.preventDefault();
      onBeforeSelect?.();
      setSelected([...items]);
      anchorId = getId(item);
      return;
    }
    if (event.key === "Escape") {
      if (dragState.dragging) return;
      event.preventDefault();
      onBeforeSelect?.();
      setSelected([]);
      anchorId = null;
      return;
    }
    if (
      [
        "ArrowLeft",
        "ArrowRight",
        "ArrowUp",
        "ArrowDown",
        "Home",
        "End",
      ].includes(event.key)
    ) {
      const current = event.currentTarget;
      const container = getScrollContainer();
      if (!(current instanceof HTMLElement) || !container) return;
      const elements = Array.from(
        container.querySelectorAll<HTMLElement>('[role="option"]'),
      );
      const index = items.findIndex((value) => getId(value) === getId(item));
      let next =
        event.key === "Home"
          ? 0
          : event.key === "End"
            ? items.length - 1
            : index +
              (event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 1);
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        const currentRect = current.getBoundingClientRect();
        const candidates = elements
          .map((element, position) => ({
            position,
            rect: element.getBoundingClientRect(),
          }))
          .filter(({ rect }) =>
            event.key === "ArrowUp"
              ? rect.top < currentRect.top
              : rect.top > currentRect.top,
          );
        candidates.sort(
          (a, b) =>
            Math.abs(a.rect.top - currentRect.top) -
              Math.abs(b.rect.top - currentRect.top) ||
            Math.abs(a.rect.left - currentRect.left) -
              Math.abs(b.rect.left - currentRect.left),
        );
        if (candidates.length) next = candidates[0].position;
      }
      next = Math.max(0, Math.min(items.length - 1, next));
      if (!items[next] || !elements[next]) return;
      event.preventDefault();
      focusId = getId(items[next]);
      elements[next].focus();
      if (event.shiftKey) {
        if (anchorId === null) anchorId = getId(item);
        onClick(items[next], new MouseEvent("click", { shiftKey: true }));
      }
      return;
    }
    handleGridItemKeyDown(
      item,
      event,
      onClick as (i: T, e: MouseEvent | KeyboardEvent) => void,
      true,
    );
  }

  function toggle(item: T, checked: boolean) {
    onBeforeSelect?.();
    anchorId = getId(item);
    if (checked) {
      setSelected([...getSelected(), item]);
    } else {
      setSelected(getSelected().filter((x) => getId(x) !== getId(item)));
    }
  }

  function reconcileFocus() {
    const items = getItems();
    if (focusId === null || items.some((item) => getId(item) === focusId))
      return;
    focusId = items.length ? getId(items[0]) : null;
    if (focusWithin && items.length) {
      getScrollContainer()
        ?.querySelector<HTMLElement>('[role="option"]')
        ?.focus();
    }
  }

  return {
    get selectedSet() {
      return selectedSet;
    },
    handleScroll,
    destroy,
    onClick,
    earlyOnClick,
    onRightClick,
    onKeyDown,
    onFocus: (item: T) => {
      focusWithin = true;
      focusId = getId(item);
    },
    onFocusOut: (event: FocusEvent) => {
      if (
        !(event.relatedTarget instanceof Node) ||
        !getScrollContainer()?.contains(event.relatedTarget)
      )
        focusWithin = false;
    },
    reconcileFocus,
    tabIndex: (item: T) =>
      getId(item) === (focusId ?? (getItems()[0] && getId(getItems()[0])))
        ? 0
        : -1,
    toggle,
  };
}
