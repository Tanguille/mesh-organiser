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
