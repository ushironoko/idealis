export const createWasm = async (bytes: Uint8Array) => {
  const module = await WebAssembly.compile(new Uint8Array(bytes));
  const imports = WebAssembly.Module.imports(module);
  if (imports.length !== 0)
    throw new Error("SAT candidate requires host imports");
  const instance = await WebAssembly.instantiate(module, {});
  const memory = instance.exports.memory;
  if (!(memory instanceof WebAssembly.Memory))
    throw new Error("Missing SAT memory");
  const call = (name: string, ...args: number[]) => {
    const method = instance.exports[name];
    if (typeof method !== "function") throw new Error(`Missing ${name}`);
    return Number(method(...args));
  };
  if (call("sat_abi_version") !== 1) throw new Error("Wrong SAT ABI version");
  return {
    imports,
    request: (input: unknown): unknown => {
      const encoded = new TextEncoder().encode(JSON.stringify(input));
      const pointer = call("sat_input_resize", encoded.length);
      new Uint8Array(memory.buffer, pointer, encoded.length).set(encoded);
      call("sat_dispatch");
      return JSON.parse(
        new TextDecoder().decode(
          new Uint8Array(
            memory.buffer,
            call("sat_output_ptr"),
            call("sat_output_len"),
          ),
        ),
      );
    },
  };
};
