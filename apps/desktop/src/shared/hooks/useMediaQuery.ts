import * as React from "react";

/** Subscribe to a CSS media query; SSR/first paint uses `defaultMatches`. */
export function useMediaQuery(
  query: string,
  defaultMatches = false,
): boolean {
  const getMatches = React.useCallback(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return defaultMatches;
    }
    return window.matchMedia(query).matches;
  }, [defaultMatches, query]);

  const [matches, setMatches] = React.useState(getMatches);

  React.useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return;
    }
    const mql = window.matchMedia(query);
    const onChange = () => setMatches(mql.matches);
    onChange();
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, [query]);

  return matches;
}

export function useIsNarrowViewport(breakpointPx: number): boolean {
  return useMediaQuery(`(max-width: ${breakpointPx - 1}px)`);
}
