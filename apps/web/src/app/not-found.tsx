import Link from "next/link";

// Note: at the root level (outside [locale]) we can't use next-intl
// translations, so this page is bilingual via plain JSX.

export default function NotFound() {
  return (
    <div
      style={{
        minHeight: "100vh",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "2rem",
        fontFamily:
          "-apple-system, BlinkMacSystemFont, 'SF Pro Display', Inter, system-ui, sans-serif",
      }}
    >
      <div style={{ maxWidth: 460, textAlign: "center" }}>
        <p
          style={{
            fontFamily: "'JetBrains Mono', SF Mono, monospace",
            fontSize: 12,
            letterSpacing: "0.18em",
            textTransform: "uppercase",
            opacity: 0.55,
            margin: 0,
          }}
        >
          404
        </p>
        <h1
          style={{
            fontSize: "clamp(2rem, 4vw, 2.75rem)",
            fontWeight: 500,
            letterSpacing: "-0.02em",
            margin: "1rem 0 0.5rem",
          }}
        >
          Página no encontrada / Page not found
        </h1>
        <p style={{ opacity: 0.7, lineHeight: 1.5 }}>
          La URL que has solicitado no existe.
          <br />
          The URL you requested does not exist.
        </p>
        <p style={{ marginTop: "2.5rem" }}>
          <Link
            href="/es/"
            style={{
              display: "inline-block",
              padding: "0.75rem 1.25rem",
              fontFamily: "'JetBrains Mono', SF Mono, monospace",
              fontSize: 13,
              textDecoration: "none",
              color: "currentColor",
              border: "1px solid currentColor",
              marginRight: 8,
            }}
          >
            Ir al inicio →
          </Link>
          <Link
            href="/en/"
            style={{
              display: "inline-block",
              padding: "0.75rem 1.25rem",
              fontFamily: "'JetBrains Mono', SF Mono, monospace",
              fontSize: 13,
              textDecoration: "none",
              color: "currentColor",
              border: "1px solid currentColor",
            }}
          >
            Go to home →
          </Link>
        </p>
      </div>
    </div>
  );
}
