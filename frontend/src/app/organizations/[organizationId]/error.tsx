"use client";

export default function ErrorPage({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return <div className="errorState"><p className="eyebrow">Gateway request failed</p><h1>We could not load this view.</h1><p>{error.message}</p><button className="primaryButton" onClick={reset}>Try again</button></div>;
}
