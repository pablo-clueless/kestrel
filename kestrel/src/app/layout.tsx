import { ThemeProvider } from "next-themes";
import localFont from "next/font/local";
import type { Metadata } from "next";

import { Providers } from "@/components/providers";
import { Toaster } from "@/components/ui/sonner";
import "./globals.css";

// Self-hosted (latin subset, variable weight) rather than next/font/google: Turbopack's Google font
// loader failed clean Docker builds ("next/font/google queries have exactly one entry"), and this
// keeps the build off the network.
const dm_sans = localFont({
  src: "./fonts/dm-sans-latin.woff2",
  variable: "--font-dm-sans",
  weight: "100 1000",
  display: "swap",
});

const jetbrains_mono = localFont({
  src: "./fonts/jetbrains-mono-latin.woff2",
  variable: "--font-jetbrains-mono",
  weight: "100 800",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Kestrel",
  description: "Kestrel",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html
      lang="en"
      className={`${dm_sans.variable} ${jetbrains_mono.variable} h-full antialiased`}
      // globals.css scrolls smoothly; this tells Next to turn that off during route changes.
      data-scroll-behavior="smooth"
      // next-themes sets the theme class and color-scheme on <html> before React hydrates.
      suppressHydrationWarning
    >
      <body className="flex min-h-full flex-col">
        <ThemeProvider attribute="class" defaultTheme="light">
          <Providers>
            {children}
            <Toaster position="top-right" richColors />
          </Providers>
        </ThemeProvider>
      </body>
    </html>
  );
}
