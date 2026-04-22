import { vi } from "vitest";

type HoistedFactory<T> = () => T;

if (typeof (vi as typeof vi & { hoisted?: unknown }).hoisted !== "function") {
  (vi as typeof vi & { hoisted: <T>(factory: HoistedFactory<T>) => T }).hoisted = <T>(
    factory: HoistedFactory<T>,
  ) => factory();
}
