import { useMemo } from "react";
import qrcode from "qrcode-generator";

/**
 * A QR code, drawn as one SVG path.
 *
 * The aha moment in onboarding.md section 1 is a phone camera pointed at a
 * screen, so this is product-critical rather than decoration, and it is worth
 * the notes:
 *
 * - Encoding happens here rather than over the socket. The URL is already in
 *   hand and a QR is a rendering of it, not daemon state.
 * - The modules stay dark on light in both themes (`--qr` does not invert).
 *   Some scanners refuse an inverted code, and a phone camera wants contrast
 *   more than the window wants consistency.
 * - Error correction is M, the usual balance. A screen is a clean surface, so
 *   the extra redundancy of Q or H only costs density.
 */
export function Qr({
  value,
  size = 148,
  quiet = 2,
  title,
}: {
  value: string;
  size?: number;
  /** Modules of margin. The spec asks for 4; 2 is enough on a screen. */
  quiet?: number;
  title?: string;
}) {
  const { path, extent } = useMemo(() => {
    // Type 0 lets the library pick the smallest version that fits.
    const qr = qrcode(0, "M");
    qr.addData(value);
    qr.make();

    const count = qr.getModuleCount();
    const parts: string[] = [];
    for (let row = 0; row < count; row++) {
      for (let column = 0; column < count; column++) {
        if (qr.isDark(row, column)) {
          // One path with many subpaths beats one rect per module: a dense code
          // is ~1200 modules, and that many DOM nodes is visibly slow.
          parts.push(`M${column + quiet} ${row + quiet}h1v1h-1z`);
        }
      }
    }
    return { path: parts.join(""), extent: count + quiet * 2 };
  }, [value, quiet]);

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${extent} ${extent}`}
      shapeRendering="crispEdges"
      role="img"
      aria-label={title ?? `QR code for ${value}`}
      style={{ display: "block", borderRadius: 6, background: "#fff" }}
    >
      <rect width={extent} height={extent} fill="#fff" />
      <path d={path} fill="var(--qr)" />
    </svg>
  );
}
