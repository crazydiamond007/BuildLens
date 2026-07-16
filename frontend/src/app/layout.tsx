import type { Metadata } from "next";
import "@fontsource-variable/hanken-grotesk";
import "@fontsource-variable/jetbrains-mono";
import "./globals.css";

export const metadata: Metadata = {
  title: { default: "BuildLens", template: "%s | BuildLens" },
  description: "GitHub Actions analytics, DORA metrics, and grounded build intelligence.",
};

const themeScript = `
try {
  const saved = localStorage.getItem("buildlens-theme");
  document.documentElement.dataset.theme = saved === "light" ? "light" : "dark";
} catch (_) {}
`;

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" data-theme="dark" suppressHydrationWarning>
      <head><script dangerouslySetInnerHTML={{ __html: themeScript }} /></head>
      <body>{children}</body>
    </html>
  );
}
