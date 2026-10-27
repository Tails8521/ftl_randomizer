/* tslint:disable */
/* eslint-disable */
/**
 * Read an `ftl.dat` and draw the 28 player ships: their rooms, their loadouts
 * and their colors. `seed` is a whole number; the same number draws the same
 * ships.
 */
export function open(raw: Uint8Array, seed: number): Job;
/**
 * An archive that was loaded, with the ships already drawn.
 */
export class Job {
  private constructor();
  free(): void;
  /**
   * The whole patched `ftl.dat`.
   *
   * The ships are drawn again from the same seed, which draws the same ships,
   * so the two files always agree. It is the slow call: it reads the archive
   * a second time and repacks all of it.
   */
  dat(): Uint8Array;
  /**
   * Bytes of the `.ftl` mod for a mod manager.
   */
  ftl(): Uint8Array;
  /**
   * The seed the ships were drawn from.
   */
  seed(): number;
  /**
   * Draw the ships again from another seed, with the archive still loaded, so
   * a person who does not like the ships only has to type a number.
   */
  reseed(seed: number): void;
  /**
   * How many files of the game changed.
   */
  changed(): number;
  /**
   * Anything the run thought was worth saying.
   */
  warnings(): string[];
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_job_free: (a: number, b: number) => void;
  readonly job_changed: (a: number) => number;
  readonly job_dat: (a: number, b: number) => void;
  readonly job_ftl: (a: number, b: number) => void;
  readonly job_reseed: (a: number, b: number, c: number) => void;
  readonly job_seed: (a: number) => number;
  readonly job_warnings: (a: number, b: number) => void;
  readonly open: (a: number, b: number, c: number, d: number) => void;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_export_0: (a: number, b: number) => number;
  readonly __wbindgen_export_1: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
