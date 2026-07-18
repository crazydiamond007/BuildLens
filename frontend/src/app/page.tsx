import Link from "next/link";
import { redirect } from "next/navigation";

import { ApiError, getMe, githubLoginUrl } from "@/lib/api";

const signInErrors: Record<string, string> = {
  access_denied: "Sign-in was cancelled on GitHub. Nothing was shared.",
  expired_state: "That sign-in attempt expired before it finished. Try again.",
  invalid_request: "That sign-in response could not be verified. Try again.",
};

export default async function Home({
  searchParams,
}: {
  searchParams: Promise<{ error?: string }>;
}) {
  const { error: errorCode } = await searchParams;
  let me = null;
  try {
    me = await getMe();
  } catch (error) {
    if (!(error instanceof ApiError && error.status === 401)) throw error;
  }
  if (me?.memberships[0]) redirect(`/organizations/${me.memberships[0].id}/overview`);
  const notice = errorCode
    ? (signInErrors[errorCode] ?? signInErrors.invalid_request)
    : null;
  return me ? <LoginState signedIn email={me.email} notice={notice} /> : <LoginState notice={notice} />;
}

// BuildLens connects to GitHub in two steps, and it says so before the redirect
// rather than after. Signing in reads only an identity; the repository access is
// a separate GitHub App install, scoped to the repositories the person picks.
// Kept in sync with the App's user-authorization request (no scopes) in
// gateway/src/github.rs and its installation permissions documented in the README.
function ScopeDisclosure() {
  return (
    <div className="scopeDisclosure">
      <p className="scopeIntro">BuildLens connects to GitHub in two steps:</p>
      <ul className="scopeList">
        <li>
          <div className="scopeStep">1 &middot; Sign in</div>
          <div className="scopeKeys"><code>read:user</code><code>user:email</code></div>
          <span>Your name, avatar, and verified email address. This becomes your BuildLens account and your personal workspace. Nothing else is read when you sign in.</span>
        </li>
        <li>
          <div className="scopeStep">2 &middot; Install the app</div>
          <div className="scopeKeys"><code>actions:read</code><code>contents:read</code><code>metadata:read</code></div>
          <span>On only the repositories you choose, BuildLens reads workflow runs, jobs, and logs, plus commit metadata, and receives their build events through a GitHub App. You pick the repositories on GitHub and can change them at any time.</span>
        </li>
      </ul>
      <p className="scopeNote">
        BuildLens never pushes code, opens pull requests, or changes your workflows. Access is limited to the repositories you install the app on &mdash; not your whole account &mdash; and tokens are encrypted before storage and never reach the browser.
      </p>
    </div>
  );
}

function LoginState({ signedIn = false, email, notice }: { signedIn?: boolean; email?: string; notice?: string | null }) {
  return (
    <main className="loginPage">
      <section className="loginPanel">
        <div className="brand loginBrand"><span className="brandMark"><i /><i /><i /></span><span>BuildLens</span></div>
        <p className="eyebrow">Engineering intelligence</p>
        <h1>See the signal behind every build.</h1>
        <p className="loginCopy">Connect GitHub Actions to track delivery performance, surface flaky tests, and turn failed builds into grounded recommendations.</p>
        {notice && <p className="loginNotice" role="status">{notice}</p>}
        {signedIn ? (
          <div className="emptyState"><h3>No organization access</h3><p>{email} is signed in, but has no active BuildLens membership.</p></div>
        ) : (
          <>
            <ScopeDisclosure />
            <Link className="primaryButton" href={githubLoginUrl()}>Continue with GitHub</Link>
          </>
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
