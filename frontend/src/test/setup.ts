import "@testing-library/jest-dom/vitest";

// React Router builds a `Request` with an `AbortSignal` for every navigation. Under jsdom the signal is
// jsdom's, which Node's `Request` rejects, so navigations would fail in tests. Tests never abort, so the
// signal is dropped.
const NodeRequest = globalThis.Request;
globalThis.Request = class extends NodeRequest {
  constructor(input: RequestInfo | URL, init?: RequestInit) {
    if (init?.signal) {
      const { signal: _signal, ...rest } = init;
      super(input, rest);
    } else {
      super(input, init);
    }
  }
} as typeof Request;
