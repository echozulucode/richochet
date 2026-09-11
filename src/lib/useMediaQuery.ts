import { useEffect, useState } from 'react';

/** Subscribe to a media query. Returns false anywhere matchMedia is unavailable. */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState<boolean>(
    () => globalThis.matchMedia?.(query).matches ?? false,
  );

  useEffect(() => {
    const list = globalThis.matchMedia?.(query);
    if (!list) return;
    const onChange = (): void => {
      setMatches(list.matches);
    };
    onChange();
    list.addEventListener('change', onChange);
    return () => {
      list.removeEventListener('change', onChange);
    };
  }, [query]);

  return matches;
}
