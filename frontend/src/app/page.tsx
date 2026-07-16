import Link from "next/link";
import { redirect } from "next/navigation";

import { ApiError, getMe, githubLoginUrl } from "@/lib/api";

export default async function Home() {
  let me = null;
  try {
    me = await getMe();
  } catch (error) {
    if (!(error instanceof ApiError && error.status === 401)) throw error;
  }
  if (me?.memberships[0]) redirect(`/organizations/${me.memberships[0].id}/overview`);
  return me ? <LoginState signedIn email={me.email} /> : <LoginState />;
}

function LoginState({ signedIn = false, email }: { signedIn?: boolean; email?: string }) {
  return (
    <main className="loginPage">
      <section className="loginPanel">
        <div className="brand loginBrand"><span className="brandMark"><i /><i /><i /></span><span>BuildLens</span></div>
        <p className="eyebrow">Engineering intelligence</p>
        <h1>See the signal behind every build.</h1>
        <p className="loginCopy">Connect GitHub Actions to track delivery performance, surface flaky tests, and turn failed builds into grounded recommendations.</p>
        {signedIn ? (
          <div className="emptyState"><h3>No organization access</h3><p>{email} is signed in, but has no active BuildLens membership.</p></div>
        ) : (
          <Link className="primaryButton" href={githubLoginUrl()}>Continue with GitHub</Link>
        )}
        <div className="loginFeatures"><span>DORA four keys</span><span>Build scoring</span><span>Failure analysis</span></div>
      </section>
      <aside className="loginVisual" aria-hidden="true">
        <div className="visualGrid" />
        <div className="floatingMetric metricOne"><small>Deployment frequency</small><strong>3.8 / day</strong><span>High performance</span></div>
        <div className="floatingMetric metricTwo"><small>Build health</small><strong>92</strong><span>18 repositories</span></div>
        <div className="floatingMetric metricThree"><small>Latest analysis</small><strong>Cache miss in test stage</strong><span>Evidence linked to job and step</span></div>
      </aside>
    </main>
  );
}
