/**
 * Gate list enter-animations so only *newly appended* rows animate.
 * First paint / bulk history load must not start every row at opacity 0
 * (that blanks the whole list for the animation duration).
 */

/**
 * Given the previous known id set and the next id list, return:
 * - `nextSeen`: updated set of all known ids
 * - `animateIds`: ids that should play enter animation this frame
 *
 * When `prevSeen` is empty, treat as initial load: mark all seen, animate none.
 */
export function nextEnterAnimationIds(
  prevSeen: ReadonlySet<string>,
  nextIds: readonly string[],
): { nextSeen: Set<string>; animateIds: Set<string> } {
  if (prevSeen.size === 0) {
    return {
      nextSeen: new Set(nextIds),
      animateIds: new Set(),
    };
  }

  const animateIds = new Set<string>();
  const nextSeen = new Set(prevSeen);
  for (const id of nextIds) {
    if (!nextSeen.has(id)) {
      animateIds.add(id);
      nextSeen.add(id);
    }
  }
  return { nextSeen, animateIds };
}
