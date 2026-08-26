const HEX_COLOR = /^#?([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i;

type Rgb = {
  blue: number;
  green: number;
  red: number;
};

function parseHexColor(value: string): Rgb {
  const match = HEX_COLOR.exec(value.trim());
  if (!match) {
    throw new Error(`Expected a six-digit hex color, received: ${value}`);
  }

  return {
    red: Number.parseInt(match[1], 16),
    green: Number.parseInt(match[2], 16),
    blue: Number.parseInt(match[3], 16),
  };
}

function linearize(channel: number): number {
  const normalized = channel / 255;
  return normalized <= 0.04045
    ? normalized / 12.92
    : ((normalized + 0.055) / 1.055) ** 2.4;
}

export function relativeLuminance(color: string): number {
  const { blue, green, red } = parseHexColor(color);
  return (
    0.2126 * linearize(red) +
    0.7152 * linearize(green) +
    0.0722 * linearize(blue)
  );
}

export function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}
