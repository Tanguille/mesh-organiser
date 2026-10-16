import { normalizeFileTypeFilter, type FileType } from "./blob_api";

// The filter fields every paged stream exposes setters for; the model and
// group filters extend it with their own id/flag fields.
export interface StreamFilter<O> {
  orderBy: O;
  textSearch: string | null;
  fileTypes: FileType[] | null;
}

/**
 * Base for stream managers backed by a regenerable async page generator.
 *
 * Subclasses supply {@link makeGenerator}; the search/order/file-type setters
 * and the paging {@link fetch} are shared. Subclass constructors must set
 * their own fields and then call `this.regenerate()` — the base cannot,
 * because its abstract {@link makeGenerator} reads subclass state that is not
 * initialised until after `super()` returns.
 */
export abstract class GeneratorStreamManager<
  T,
  F extends StreamFilter<unknown>,
> {
  // Own copy so setter mutations never leak into the caller's filter object.
  protected filter: F;
  protected generator: AsyncGenerator<T[]> | null = null;

  protected constructor(filter: F) {
    this.filter = { ...filter };
  }

  protected abstract makeGenerator(): AsyncGenerator<T[]>;

  protected regenerate(): void {
    this.generator = this.makeGenerator();
  }

  setSearchText(text: string | null): void {
    this.filter.textSearch = text;
    this.regenerate();
  }

  setOrderBy(order_by: F["orderBy"]): void {
    this.filter.orderBy = order_by;
    this.regenerate();
  }

  setFileTypes(fileTypes: FileType[]): void {
    this.filter.fileTypes = normalizeFileTypeFilter(fileTypes);
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
