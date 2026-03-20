type Constructor<T = unknown> = new (...args: unknown[]) => T;
type Token<T = unknown> = Constructor<T> | symbol | string;

export interface IDependencyContainer {
  require<T>(type: Token<T>): T;
  optional<T>(type: Token<T>): T | null;
  addSingleton<T>(obj: T): void;
  addSingleton<T>(token: Token<T>, obj: T): void;
  clear(): void;
}

export class DependencyContainer implements IDependencyContainer {
  private container: Map<
    symbol | string | (new (...args: unknown[]) => unknown),
    unknown
  > = new Map();

  require<T>(type: Token<T>): T {
    const instance = this.container.get(type);
    if (instance === undefined) {
      throw new Error(`Dependency not found: ${String(type)}`);
    }
    return instance as T;
  }

  optional<T>(type: Token<T>): T | null {
    const instance = this.container.get(type);
    return (instance !== undefined ? instance : null) as T | null;
  }

  addSingleton<T>(obj: T): void;
  addSingleton<T>(token: Token<T>, obj: T): void;
  addSingleton<T>(tokenOrObj: Token<T> | T, obj?: T): void {
    if (obj !== undefined) {
      // Token-based: addSingleton<IMyInterface>(IMyInterfaceToken, myInstance)
      this.container.set(
        tokenOrObj as symbol | string | (new (...args: unknown[]) => unknown),
        obj,
      );
    } else {
      // Class-based: addSingleton(myInstance)
      const instance = tokenOrObj as T;
      const constructor = (
        instance as object & {
          constructor: new (...args: unknown[]) => unknown;
        }
      ).constructor;
      this.container.set(constructor, instance);
    }
  }

  clear(): void {
    this.container.clear();
  }
}

const container = new DependencyContainer();

export function getContainer(): IDependencyContainer {
  return container;
}

export function resetContainer(): void {
  container.clear();
}
