/** Empty shell when no conversation is selected (outer nav owns selection). */
export function TimelineEmpty() {
  return (
    <div className="flex h-full min-h-0 flex-1 items-center justify-center bg-surface text-[13px] text-ink-muted">
      Select a conversation or create one to start.
    </div>
  );
}
