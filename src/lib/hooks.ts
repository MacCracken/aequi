import { useEffect, useRef } from "react";

/**
 * Ref that is true while the component is mounted, for dropping the result of
 * a request that finishes after the component went away.
 */
export function useMountedRef() {
  const mounted = useRef(true);
  useEffect(() => {
    // Set on every mount: StrictMode runs effects (and cleanups) twice.
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  return mounted;
}
