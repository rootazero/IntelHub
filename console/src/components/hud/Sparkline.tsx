/** Hand-rolled SVG sparkline — zero dependencies (Crucix-style). */
export default function Sparkline({
  points,
  width = 120,
  height = 28,
  color = "#64f0c8",
}: {
  points: number[];
  width?: number;
  height?: number;
  color?: string;
}) {
  if (points.length < 2) return <svg width={width} height={height} />;
  const min = Math.min(...points);
  const max = Math.max(...points);
  const span = max - min || 1;
  const step = width / (points.length - 1);
  const d = points
    .map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)},${(height - 2 - ((v - min) / span) * (height - 4)).toFixed(1)}`)
    .join(" ");
  return (
    <svg width={width} height={height} className="block">
      <path d={d} fill="none" stroke={color} strokeWidth={1.4} strokeLinejoin="round" />
      <circle
        cx={width}
        cy={height - 2 - ((points[points.length - 1] - min) / span) * (height - 4)}
        r={2}
        fill={color}
      />
    </svg>
  );
}
