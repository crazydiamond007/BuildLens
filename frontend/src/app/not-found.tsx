import Link from "next/link";

export default function NotFound() {
  return <main className="standaloneState"><p className="eyebrow">404</p><h1>That BuildLens view does not exist.</h1><p>The resource may have been removed or you may not have access.</p><Link className="primaryButton" href="/">Return home</Link></main>;
}
