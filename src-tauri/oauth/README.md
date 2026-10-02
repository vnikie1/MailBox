# Built-in sign-in applications

Google and Microsoft only let a **registered application** sign anyone in. A Halcyon build
can carry one for each, so that choosing Google or Microsoft in the account assistant just
works — nobody has to visit a cloud console.

The values are **build inputs, never committed**. The source is public, and a secret in
public source is not a secret (docs/05 §9). `build.rs` reads them from, in order:

1. environment variables of the same names — for CI;
2. `clients.env` in this directory — **gitignored**, for a developer machine.

A build with neither has no built-in client and behaves exactly as a build from a clean
checkout should: the Google and Microsoft tiles say they need setting up, and Settings →
Accounts → Sign-in applications takes a client the user registered themselves. A client
entered there always wins over a built-in one.

## Setting it up

```text
copy clients.env.example clients.env
```

Fill in `clients.env`, then build as usual (`npm run app:build`). Creating or editing the file
triggers a rebuild on its own. The build prints a warning if a value looks wrong; it never
prints the secret.

| Variable                       | Where it comes from                                                 |
| ------------------------------ | ------------------------------------------------------------------- |
| `HALCYON_GOOGLE_CLIENT_ID`     | Google Cloud → APIs & Services → Credentials → your **Desktop app** |
| `HALCYON_GOOGLE_CLIENT_SECRET` | The same client. **Required** — see below                           |
| `HALCYON_MICROSOFT_CLIENT_ID`  | Entra → App registrations → your app → Overview → Application ID    |

**Google needs both halves or neither.** Google refuses a Desktop client's token exchange
without its secret, so an ID on its own would light up the Google tile, send the user through
a full browser consent, and fail at the very last step. The build refuses to compile in half a
pair and warns instead.

**Microsoft has no secret.** A public client that sends one is rejected.

## Registering the Google application

1. [console.cloud.google.com](https://console.cloud.google.com) → create or pick a project.
2. **APIs & Services → Library** → enable the **Gmail API**. IMAP does not use it — the scope
   is what grants IMAP — but enabling it is what makes the Gmail scopes selectable on the
   consent screen.
3. **OAuth consent screen** → External. Add the scope `https://mail.google.com/`.
4. **Credentials → Create credentials → OAuth client ID** → application type **Desktop app**.
   Not "Web application": Halcyon listens on a random loopback port, and only Desktop clients
   accept one.

`https://mail.google.com/` is a **restricted** scope. While the app's publishing status is
**Testing**, only the accounts listed under **Test users** (up to 100) can sign in, and Google
expires their refresh tokens after seven days — which looks, from inside the app, like being
signed out once a week. See [Publishing the Google application](#publishing-the-google-application).

## Publishing the Google application

Google's console calls the consent-screen settings **Google Auth Platform** (older consoles:
_APIs & Services → OAuth consent screen_), with pages named **Branding**, **Audience**,
**Clients**, **Data Access** and **Verification Center**. There are two ways out of Testing, and
they are very different amounts of work.

### A. Personal use — publish without verification (minutes)

For your own accounts and a handful of others. Google's _"When is verification not needed"_ page
lists personal use as an exception: the app keeps working unverified, and its users click
through a warning.

1. **Branding** → set the app name to **Halcyon** and fill in the support and developer contact
   emails. Leave the logo empty for now: Google does not show one until the brand is verified.
2. **Audience** → _Publishing status: Testing_ → **Publish app** → confirm. Google will say the
   app needs verification because of its restricted scope; it stays usable without it.
3. **Then** add the account in Halcyon. A refresh token issued while the app was still in
   Testing keeps its seven-day life, so an account added before publishing should be signed in
   again once afterwards.

What sign-in then looks like: _"Google hasn't verified this app"_ → **Advanced** →
**Go to Halcyon (unsafe)** → the normal consent screen.

| Limit                          | Testing                     | Published, unverified                                                   |
| ------------------------------ | --------------------------- | ----------------------------------------------------------------------- |
| Who can sign in                | Listed test users only      | Any Google account                                                      |
| Refresh tokens                 | Expire after 7 days         | Do not expire on that schedule                                          |
| Warning screen                 | Yes                         | Yes, at every consent                                                   |
| User cap                       | 100 test users              | 100 **new** users over the project's **lifetime** — it cannot be reset |
| Workspace accounts             | Per admin policy            | Blocked where the admin disallows unverified apps                       |

### B. Public distribution — verify the app (weeks)

Removes the warning screen and the 100-user cap.

**Prepared on 2026-10-03, not yet submitted:**

| Step | State |
| --- | --- |
| 1. Domain | **Done.** `https://vnikie1.github.io/` is verified in Search Console (URL-prefix property, account vnikie1@gmail.com) by the file `googlebf2615cb1c964bad.html` in the repository `vnikie1/vnikie1.github.io`, made for the purpose. **Do not delete that file**: Google re-checks it. A _Domain_ property was not possible — it needs a DNS record, and `github.io` is GitHub's. Homepage and privacy policy are live and say what Google asks for (privacy policy 1.2, _Google accounts_) |
| 2. Branding | Logo ready: `consent-logo-120x120.png`, beside this file — the designer's 300 px Store icon, downscaled. **Not uploaded**: the Branding page says uploading a logo means the app must then be submitted for verification, so it goes up with the submission |
| 3. Data Access | Nothing declared yet, which is why the Verification Center says data-access verification "is not required". Declaring `https://mail.google.com/` is what starts the restricted-scope review |
| 4. Demo video | To record — the shot list is under step 4 below. It needs a real Google sign-in and the publisher's YouTube account |
| 5. Submit | Not started |

What Google asks for:

1. **A domain you own**, verified in Google Search Console by an owner or editor of the Cloud
   project, hosting:
   - a **homepage** that describes Halcyon — not only a download or sign-in page;
   - a **privacy policy** (`PRIVACY.md` is the starting point). It must say how Google user data is
     accessed, used, stored and shared, and include the Limited Use statement: _"Halcyon's use and
     transfer of information received from Google APIs will adhere to the Google API Services User
     Data Policy, including the Limited Use requirements."_
   - terms of service (optional, recommended).
2. **Branding** → app name (it may not contain "Google" or "Gmail"), a 120×120 logo, the three
   links above, and the domain under **Authorized domains**. This goes to _brand verification_,
   typically a few business days.
3. **Data Access** → `https://mail.google.com/`, with a justification. Reviewers often push
   Gmail apps towards narrower Gmail **API** scopes such as `gmail.modify`; lead with why those
   cannot work here:
   > Halcyon is an IMAP and SMTP mail client. Google's IMAP and SMTP servers accept OAuth only
   > with the `https://mail.google.com/` scope (SASL XOAUTH2); the narrower Gmail API scopes grant
   > no IMAP or SMTP access. Halcyon also offers permanent deletion. All mail is downloaded to,
   > stored on and processed on the user's own computer; Halcyon has no server and sends no
   > telemetry.
4. **A demo video** — unlisted on YouTube, in English — showing: adding a Google account in
   Halcyon; the browser consent screen with **the client ID visible in the address bar** (it is
   the `client_id=` parameter); the app name matching the consent screen; and each use of the
   scope — reading, sending, moving, deleting and permanently deleting mail.

   A shot list that covers it, about three minutes, recorded with the Snipping Tool's screen
   recorder (Windows 11) and a test Google account rather than a personal one:

   1. Halcyon open with no Google account. **Settings → Accounts → Add Account → Google**, type
      the address, continue.
   2. The browser opens Google's sign-in. Before signing in, click into the address bar and
      scroll it slowly so the whole `client_id=…` parameter is readable; hold for a few seconds.
   3. Sign in. Google's unverified-app screen — the app name _Halcyon_ visible — then
      **Advanced → Go to Halcyon**, the consent screen naming the Gmail permission, **Continue**,
      and the browser's _Signed in_ page.
   4. Back in Halcyon: the Inbox fills. Open a message — **reading**.
   5. **Ctrl+N**, write to the account itself, **send**; show it arriving.
   6. Drag a message to another mailbox — **moving**. Delete one — **deleting**, into the Bin.
   7. Right-click the Bin → **Erase Deleted Items…** and confirm — **permanent deletion**, the
      reason the full scope is needed.
   8. Optionally, <https://myaccount.google.com/connections>, showing Halcyon among the
      third-party connections and how to remove its access.
5. **Verification Center** → submit, and expect questions by email. Budget weeks.
6. **Security assessment (CASA).** Google's policy says **local client applications — whose data
   is run, stored and processed only on the user's device — do not need one.** An app loses that
   status if it sends restricted-scope data to a developer's or third party's server without an
   explicit user action. Halcyon has no such server (standing rules 9 and 16), so state its local
   architecture plainly in the submission. Google decides; if it still asks, the assessment is done
   by an App Defense Alliance lab — Gmail usually lands at Tier 2 — and is renewed yearly. TAC
   Security lists Tier 2 at **$540–$1,800**.

Once approved, anyone can add a Google account with no warning and no cap. Keep the same client —
the builds already carry it. Changing the approved name, logo, links, domains or scopes later
means submitting again (Google's _"Changes to approved app"_ page).

**Internal** user type skips all of this, but only for accounts inside one Google Workspace
organisation, and only if the Cloud project belongs to it. It cannot cover `@gmail.com`
addresses.

## Registering the Microsoft application

1. [entra.microsoft.com](https://entra.microsoft.com) → **App registrations → New
   registration**.
2. **Supported account types**: _Accounts in any organizational directory and personal
   Microsoft accounts_. Outlook.com, Hotmail and Live addresses are personal accounts.
3. Leave the redirect URI empty for now and register.
4. **Manifest** → add `http://127.0.0.1/callback` as a public-client redirect URI. The
   manifest comes in one of two formats, and which one you see depends on the account you
   registered with:
   - **Microsoft Graph format** (a work or school tenant — the usual case): under
     `publicClient`, set

     ```json
     "publicClient": { "redirectUris": ["http://127.0.0.1/callback"] }
     ```

   - **Azure AD Graph format** (still shown for apps registered with a personal Microsoft
     account): in `replyUrlsWithType`, add

     ```json
     { "url": "http://127.0.0.1/callback", "type": "InstalledClient" }
     ```

   The portal's redirect-URI box refuses an `http://127.0.0.1` address, so the manifest is the
   only way to add it. Entra ignores the port on loopback addresses, which is what lets
   Halcyon use a different one each time; the path still has to be `/callback`. Use
   `127.0.0.1`, not `localhost` — Entra does not treat the two as the same address, and
   Halcyon only listens on `127.0.0.1`.

5. **Authentication** → _Allow public client flows_ → **Yes**.
6. **API permissions → Add a permission → APIs my organization uses → Office 365 Exchange
   Online → Delegated** → `IMAP.AccessAsUser.All` and `SMTP.Send`.
7. Copy the **Application (client) ID** from **Overview**.

No secret, and no admin consent for personal accounts. A work or school tenant may still
block third-party IMAP, or SMTP AUTH per mailbox — that is the tenant's setting, not
something Halcyon can change (docs/05 §3).
