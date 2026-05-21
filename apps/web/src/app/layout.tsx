import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import { themeScript } from "~/lib/theme";
import { SITE_NAME } from "~/lib/site";
import "../styles/globals.css";

const jetbrainsMono = localFont({
  src: [
    {
      path: "../../node_modules/@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-normal.woff2",
      weight: "400",
      style: "normal",
    },
    {
      path: "../../node_modules/@fontsource/jetbrains-mono/files/jetbrains-mono-latin-500-normal.woff2",
      weight: "500",
      style: "normal",
    },
  ],
  variable: "--font-mono-loaded",
  display: "swap",
  preload: true,
});

export const metadata: Metadata = {
  metadataBase: new URL("https://conclave.app"),
  title: { default: SITE_NAME, template: "%s" },
  applicationName: SITE_NAME,
  authors: [{ name: "Xus Badia" }],
  creator: "Xus Badia",
  formatDetection: {
    email: false,
    telephone: false,
  },
  icons: {
    // Modern browsers prefer the SVG (adapts to light/dark browser themes via
    // an embedded <style> with prefers-color-scheme). Older browsers fall back
    // to the .ico, then the 32px PNG.
    icon: [
      { url: "/favicon.svg", type: "image/svg+xml" },
      { url: "/favicon.ico", sizes: "32x32" },
      { url: "/icon-192.png", sizes: "192x192", type: "image/png" },
      { url: "/icon-512.png", sizes: "512x512", type: "image/png" },
    ],
    apple: [{ url: "/apple-touch-icon.png", sizes: "180x180" }],
    shortcut: ["/favicon.ico"],
  },
  manifest: undefined,
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#faf9f8" },
    { media: "(prefers-color-scheme: dark)", color: "#080a0c" },
  ],
  colorScheme: "light dark",
  width: "device-width",
  initialScale: 1,
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="es" suppressHydrationWarning className={jetbrainsMono.variable}>
      <head>
        <script
          dangerouslySetInnerHTML={{ __html: themeScript }}
          suppressHydrationWarning
        />
      </head>
      <body className="antialiased">{children}</body>
    </html>
  );
}
