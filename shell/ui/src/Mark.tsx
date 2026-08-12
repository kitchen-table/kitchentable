/** The Kitchen Table mark: a tabletop and two legs. */
export function Mark({ size = 32 }: { size?: number }) {
  return (
    <svg viewBox="0 0 32 32" width={size} height={size} role="img" aria-label="Kitchen Table">
      <rect x="4" y="11" width="24" height="5" rx="2.5" fill="var(--accent)" />
      <rect x="8" y="16" width="3.4" height="11" rx="1.7" fill="var(--ink)" />
      <rect x="20.6" y="16" width="3.4" height="11" rx="1.7" fill="var(--ink)" />
    </svg>
  );
}
