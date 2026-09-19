// The slice of the WebAssembly JS API the extension uses. `lib: ["ES2022"]`
// keeps DOM out of this Node extension, and the global's types ship with
// lib.dom — so the narrow surface is declared here.
declare namespace WebAssembly {
  type BufferSource = ArrayBufferView | ArrayBuffer;

  interface ImportObject {
    [module: string]: { [name: string]: object | undefined } | undefined;
  }

  class Memory {
    readonly buffer: ArrayBuffer;
  }

  interface Instance<T extends object> {
    readonly exports: T;
  }

  interface ResultObject<T extends object> {
    module: object;
    instance: Instance<T>;
  }

  function instantiate<T extends object>(
    bytes: BufferSource,
    imports?: ImportObject
  ): Promise<ResultObject<T>>;
}
