import { handleGridItemKeyDown } from "$lib/utils";

interface GridSelectionOptions<T> {
  getItems: () => T[];
  getSelected: () => T[];
  setSelected: (items: T[]) => void;
  getId: (item: T) => number;
  // Fires right before an interaction (click, right-click, checkbox toggle)
  // touches the selection — including the shift-range early-return path, so
  // callers can invalidate derived state (e.g. group-grid's split-view model
  // sub-selection) exactly where they used to.
  onBeforeSelect?: () => void;
  getScrollContainer: () => HTMLElement | undefined;
  onEndOfList?: () => void;
}

// Shared selection state machine for the model/group grids: shift-range
// select, ctrl/meta toggle, single-select with scrollIntoView, the
// mousedown-before-click suppression, right-click select, and the 1s
// end-of-list scroll polling. The selection array itself stays owned by the
// component (model-grid-inner binds it as a prop, group-grid reassigns it
// from pruning/delete paths), so everything goes through accessors.
export function createGridSelection<T>(options: GridSelectionOptions<T>) {
  const {
    getItems,
    getSelected,
    setSelected,
    getId,
    onBeforeSelect,
    getScrollContainer,
    onEndOfList,
  } = options;

  // A plain Set is fine here: it is rebuilt from scratch whenever the
  // selection changes and never mutated, so SvelteSet's mutation tracking
  // would buy nothing.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const selectedSet = $derived(new Set(getSelected().map(getId)));

  let preventOnClick = $state.raw(false);

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
  }

  function onClick(item: T, event: MouseEvent) {
    if (preventOnClick) {
      preventOnClick = false;
      return;
    }

    onBeforeSelect?.();

    const items = getItems();
    const selected = getSelected();

    if (event.shiftKey && selected.length === 1) {
      let start = items.indexOf(selected[0]);
      let end = items.indexOf(item);

      if (start === -1 || end === -1) {
        return;
      }

      if (start > end) {
        [start, end] = [end, start];
      }

      setSelected(items.slice(start, end + 1));
    } else if (event.ctrlKey || event.metaKey) {
      if (selectedSet.has(getId(item))) {
        setSelected(selected.filter((x) => getId(x) !== getId(item)));
      } else {
        setSelected([...selected, item]);
      }
    } else {
      setSelected([item]);

      setTimeout(() => {
        if (event.target instanceof HTMLElement) {
          event.target.scrollIntoView({
            behavior: "smooth",
            block: "center",
          });
        }
      }, 30);
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

    const el = event.target;
    if (el instanceof HTMLElement) {
      setTimeout(() => {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
      }, 30);
    }
  }

  function onKeyDown(item: T, event: KeyboardEvent) {
    handleGridItemKeyDown(
      item,
      event,
      onClick as (i: T, e: MouseEvent | KeyboardEvent) => void,
      true,
    );
  }

  function toggle(item: T, checked: boolean) {
    onBeforeSelect?.();
    if (checked) {
      setSelected([...getSelected(), item]);
    } else {
      setSelected(getSelected().filter((x) => getId(x) !== getId(item)));
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
    toggle,
  };
}
