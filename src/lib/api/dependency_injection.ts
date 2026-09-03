type Token = symbol | string;

export class DependencyContainer {
  private container: Map<Token, unknown> = new Map();

  require<T>(type: Token): T {
    const instance = this.container.get(type);
    if (instance === undefined) {
      throw new Error(`Dependency not found: ${String(type)}`);
    }
    return instance as T;
  }

  optional<T>(type: Token): T | null {
    const instance = this.container.get(type);
    return (instance !== undefined ? instance : null) as T | null;
  }

  addSingleton(token: Token, obj: unknown): void {
    this.container.set(token, obj);
  }

  clear(): void {
    this.container.clear();
  }
}

const container = new DependencyContainer();

export function getContainer(): DependencyContainer {
  return container;
}

export function resetContainer(): void {
  container.clear();
}
