/**
 * Base for stream managers backed by a regenerable async page generator.
 *
 * Subclasses supply {@link makeGenerator}; the search/order setters and the
 * paging {@link fetch} are shared. Subclass constructors must set their own
 * fields and then call `this.regenerate()` — the base cannot, because its
 * abstract {@link makeGenerator} reads subclass state that is not initialised
 * until after `super()` returns.
 */
export abstract class GeneratorStreamManager<T, O> {
  protected orderBy: O;
  protected textSearch: string | null = null;
  protected generator: AsyncGenerator<T[]> | null = null;

  protected constructor(orderBy: O) {
    this.orderBy = orderBy;
  }

  protected abstract makeGenerator(): AsyncGenerator<T[]>;

  protected regenerate(): void {
    this.generator = this.makeGenerator();
  }

  setSearchText(text: string | null): void {
    this.textSearch = text;
    this.regenerate();
  }

  setOrderBy(order_by: O): void {
    this.orderBy = order_by;
    this.regenerate();
  }

  async fetch(): Promise<T[]> {
    return (await this.generator!.next()).value ?? [];
  }
}

/**
 * Drains a paged endpoint page by page, prefetching the next page while the
 * consumer processes the current one. Terminates on the first empty page.
 * Shared by the model and group streams, which differ only in how a page is
 * fetched.
 */
export async function* pagedStream<T>(
  fetchPage: (page: number) => Promise<T[]>,
): AsyncGenerator<T[]> {
  let page = 1;
  let prefetchNextTask: Promise<T[]> | null = null;

  while (true) {
    prefetchNextTask ??= fetchPage(page);

    const items = await prefetchNextTask;
    if (items.length === 0) {
      break;
    }

    page += 1;
    prefetchNextTask = fetchPage(page);

    yield items;
  }
}
