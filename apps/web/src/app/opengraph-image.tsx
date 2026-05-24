import { ImageResponse } from "next/og";

export const dynamic = "force-static";

export const alt = "Conclave — A virtual committee for clinical decision support";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default async function OpenGraphImage() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          background: "#faf9f8",
          padding: "72px 80px",
          fontFamily: "'SF Pro Display', Inter, system-ui, sans-serif",
        }}
      >
        {/* Top row: brand mark + wordmark */}
        <div style={{ display: "flex", alignItems: "center", gap: 18 }}>
          <svg
            width="52"
            height="52"
            viewBox="0 0 64 64"
          >
            <rect x="5" y="5" width="54" height="54" rx="15" fill="#faf9f8" stroke="#d8d8d2" strokeWidth="3" />
            <path
              d="M43 20.5C39.9 17.4 35.8 15.5 31.4 15.5C22.5 15.5 15.5 22.8 15.5 32C15.5 41.2 22.5 48.5 31.4 48.5C35.8 48.5 39.9 46.6 43 43.5"
              fill="none"
              stroke="#121214"
              strokeWidth="7.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            <circle cx="43.5" cy="20.5" r="4.75" fill="#0e7490" />
            <circle cx="43.5" cy="43.5" r="4.75" fill="#0e7490" />
            <circle cx="32" cy="32" r="4.25" fill="#121214" />
          </svg>
          <div
            style={{
              fontFamily: "'JetBrains Mono', monospace",
              fontSize: 26,
              color: "#121214",
              letterSpacing: "-0.01em",
              fontWeight: 500,
            }}
          >
            Conclave
          </div>
        </div>

        {/* Headline */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 28,
            maxWidth: 980,
          }}
        >
          <div
            style={{
              fontSize: 84,
              lineHeight: 0.96,
              letterSpacing: "-0.035em",
              color: "#121214",
              fontWeight: 500,
              display: "flex",
              flexDirection: "column",
            }}
          >
            <span>A virtual committee.</span>
            <span style={{ color: "#646469" }}>On your machine.</span>
          </div>
          <div
            style={{
              fontSize: 24,
              lineHeight: 1.4,
              color: "#3c3c40",
              maxWidth: 780,
            }}
          >
            An AI committee that argues against itself before answering. 100%
            local. No telemetry.
          </div>
        </div>

        {/* Bottom row: tags */}
        <div
          style={{
            display: "flex",
            gap: 24,
            fontFamily: "'JetBrains Mono', monospace",
            fontSize: 16,
            letterSpacing: "0.18em",
            textTransform: "uppercase",
            color: "#646469",
          }}
        >
          <span>Local-first</span>
          <span>·</span>
          <span>No telemetry</span>
          <span>·</span>
          <span>Not a medical device</span>
        </div>
      </div>
    ),
    { ...size },
  );
}
