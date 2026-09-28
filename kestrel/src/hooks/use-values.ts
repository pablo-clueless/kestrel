import { useCallback, useReducer, useState } from "react";

interface UseValuesProps<T> {
  initialValue: T;
}

type Action<T> =
  | { type: "set"; key: keyof T; value: T[keyof T] }
  | { type: "patch"; patch: Partial<T> }
  | { type: "reset"; value: T };

const reducer = <T extends object>(state: T, action: Action<T>): T => {
  switch (action.type) {
    case "set":
      // Keep the same object when nothing changed, so consumers don't re-render.
      return Object.is(state[action.key], action.value)
        ? state
        : { ...state, [action.key]: action.value };
    case "patch":
      return { ...state, ...action.patch };
    case "reset":
      return action.value;
  }
};

/**
 * A group of related values (a form, a panel's settings) in one state object.
 *
 * ```ts
 * const { values, set, patch, reset } = useValues({ initialValue: { rate: 100, keepAlive: true } });
 * set("rate", 200);
 * patch({ rate: 50, keepAlive: false });
 * reset();
 * ```
 *
 * `set`, `patch` and `reset` keep the same identity across renders, so they're safe in effect
 * dependencies and memoised children.
 */
export const useValues = <T extends object>({ initialValue }: UseValuesProps<T>) => {
  // The first value is the reset target; later changes to `initialValue` don't move it.
  const [initial] = useState(initialValue);
  const [values, dispatch] = useReducer(reducer<T>, initial);

  const set = useCallback(
    <K extends keyof T>(key: K, value: T[K]) => dispatch({ type: "set", key, value }),
    [],
  );
  const patch = useCallback((patch: Partial<T>) => dispatch({ type: "patch", patch }), []);
  const reset = useCallback(() => dispatch({ type: "reset", value: initial }), [initial]);

  return { values, set, patch, reset };
};
