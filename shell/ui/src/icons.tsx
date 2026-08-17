/**
 * The icon set, traced from the desktop mockup.
 *
 * Every path here is the one in `uimockups/Desktop App.dc.html`; nothing is
 * invented and nothing comes from an icon library. They are stroke icons on a
 * 24-unit grid drawn in `currentColor` so a caller sets colour by setting
 * `color`, which is what the mockup's inline `stroke:var(--x)` does.
 */

type IconProps = {
  size?: number;
  /** Overrides `currentColor`. Only used where the mockup pins a colour. */
  colour?: string;
};

function Svg({
  size = 17,
  colour,
  children,
  fill = "none",
}: IconProps & { children: React.ReactNode; fill?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill={fill}
      stroke={colour ?? "currentColor"}
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ flex: "none" }}
    >
      {children}
    </svg>
  );
}

/** Four panes: the library grid. */
export function LibraryIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="3" y="3" width="7" height="7" rx="1.5" />
      <rect x="14" y="3" width="7" height="7" rx="1.5" />
      <rect x="3" y="14" width="7" height="7" rx="1.5" />
      <rect x="14" y="14" width="7" height="7" rx="1.5" />
    </Svg>
  );
}

/** A heartbeat trace: the access log. */
export function ActivityIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <polyline points="3 12 8 12 11 4 14 20 17 12 21 12" />
    </Svg>
  );
}

/** Two sliders. */
export function SettingsIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <line x1="4" y1="8" x2="20" y2="8" />
      <line x1="4" y1="16" x2="20" y2="16" />
      <circle cx="9" cy="8" r="2.6" fill="currentColor" stroke="none" />
      <circle cx="15" cy="16" r="2.6" fill="currentColor" stroke="none" />
    </Svg>
  );
}

// The mockup's TeamIcon lived here and is gone with the rail row it drew. Team
// is a cloud feature (CLAUDE.md rule 10); an icon kept for a surface that must
// not be built in this repo is an invitation to build it.

/** A padlock: Private, and the "encrypted" badge. */
export function LockIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="5" y="11" width="14" height="9" rx="2" />
      <path d="M8 11V7.5a4 4 0 0 1 8 0V11" />
    </Svg>
  );
}

/** Radio waves: Household. */
export function NetworkIcon({ colour, ...rest }: IconProps) {
  return (
    <Svg colour={colour} {...rest}>
      <path d="M4 11a12 12 0 0 1 16 0" />
      <path d="M7.5 14.5a7 7 0 0 1 9 0" />
      <circle cx="12" cy="18.5" r="1.1" fill={colour ?? "currentColor"} stroke="none" />
    </Svg>
  );
}

/** An envelope: Invited. */
export function InvitedIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="3" y="5" width="18" height="14" rx="2.5" />
      <polyline points="3.5 7 12 13 20.5 7" />
    </Svg>
  );
}

/** A globe: Public. */
export function PublicIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="9" />
      <line x1="3" y1="12" x2="21" y2="12" />
      <ellipse cx="12" cy="12" rx="4" ry="9" />
    </Svg>
  );
}

/** A handset: a phone or a tablet. */
export function DeviceIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="6" y="2.5" width="12" height="19" rx="3" />
      <line x1="10" y1="18" x2="14" y2="18" />
    </Svg>
  );
}

/** A lid and a desk: anything that is not a handset. */
export function LaptopIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="4" y="5" width="16" height="11" rx="2" />
      <line x1="2" y1="20" x2="22" y2="20" />
    </Svg>
  );
}

/**
 * Whichever of the two a device is, at the size that shape wants.
 *
 * They are not interchangeable at one size: the handset is a tall narrow
 * outline and the laptop a wide short one, so the mockup draws the laptop two
 * units larger for the pair to carry the same weight in a column. `size` is the
 * handset's; the laptop takes care of itself.
 */
export function DeviceShapeIcon({
  shape,
  size = 17,
  colour,
}: IconProps & { shape: "phone" | "laptop" }) {
  return shape === "phone" ? (
    <DeviceIcon size={size} colour={colour} />
  ) : (
    <LaptopIcon size={size + 2} colour={colour} />
  );
}

/** Two arrows chasing each other: data kept in step everywhere. */
export function SyncIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 12a8 8 0 0 1 13.7-5.7L20 8" />
      <path d="M20 4v4h-4" />
      <path d="M20 12a8 8 0 0 1-13.7 5.7L4 16" />
      <path d="M4 20v-4h4" />
    </Svg>
  );
}

/** A screen on a stand: one device keeping its own copy. */
export function DesktopIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <line x1="8" y1="20" x2="16" y2="20" />
      <line x1="12" y1="16" x2="12" y2="20" />
    </Svg>
  );
}

/** A cloud with an arrow into it: a copy kept on this machine. */
export function BackupIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 15a5 5 0 0 1 .9-9.9A6.5 6.5 0 0 1 18 6.5 4.5 4.5 0 0 1 19 15" />
      <line x1="12" y1="12" x2="12" y2="21" />
      <polyline points="8.5 17.5 12 21 15.5 17.5" />
    </Svg>
  );
}

/** A crescent: the appearance toggle. */
export function MoonIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M20 14.5A8 8 0 1 1 9.5 4 6.3 6.3 0 0 0 20 14.5z" />
    </Svg>
  );
}

export function SearchIcon({ size = 13, colour }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke={colour ?? "currentColor"}
      strokeWidth={2.2}
      strokeLinecap="round"
      aria-hidden="true"
      style={{ flex: "none" }}
    >
      <circle cx="10.5" cy="10.5" r="6.5" />
      <line x1="15.5" y1="15.5" x2="21" y2="21" />
    </svg>
  );
}

export function ChevronUpIcon({ size = 14, colour }: IconProps) {
  return (
    <Svg size={size} colour={colour}>
      <polyline points="6 15 12 9 18 15" />
    </Svg>
  );
}

export function ChevronDownIcon({ size = 14, colour }: IconProps) {
  return (
    <Svg size={size} colour={colour}>
      <polyline points="6 9 12 15 18 9" />
    </Svg>
  );
}
