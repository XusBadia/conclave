import type { Metadata } from "next";

// Root page redirects to the user's preferred locale.
// Static export friendly: ships a tiny inline script for instant redirect,
// plus a <meta http-equiv="refresh"> fallback for visitors with JS disabled.
// React 19 hoists <meta> and <script> elements automatically into <head>.

export const metadata: Metadata = {
  title: "Conclave",
  robots: { index: false, follow: false },
};

const REDIRECT_SCRIPT = `(() => {
  try {
    const lang = (navigator.language || 'es').toLowerCase();
    const target = lang.startsWith('en') ? '/en/' : '/es/';
    location.replace(target);
  } catch (_) {
    location.replace('/es/');
  }
})();`;

export default function RootRedirect() {
  return (
    <>
      <meta httpEquiv="refresh" content="0;url=/es/" />
      <link rel="canonical" href="https://conclave.app/es/" />
      <script
        dangerouslySetInnerHTML={{ __html: REDIRECT_SCRIPT }}
        suppressHydrationWarning
      />
      <noscript>
        <p style={{ padding: 24, fontFamily: "system-ui", textAlign: "center" }}>
          <a href="/es/" style={{ color: "currentColor" }}>
            Continuar a Conclave / Continue to Conclave →
          </a>
        </p>
      </noscript>
    </>
  );
}
