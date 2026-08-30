import styles from './settings.module.css'

/**
 * What this app does not do. docs/06 Phase 11, standing rule 16.
 *
 * A list of absences is an odd thing to put in a settings window, and it is here because the
 * absences are the feature. Every claim below is enforced somewhere other than this file — the
 * network layer lives entirely in the Rust core, there is no analytics dependency in either
 * manifest, and `tests/secrets.rs` asserts that no error type in the app can print a
 * credential. Nothing here is a promise about future behaviour; it is a description of the
 * build the reader is running.
 *
 * If any line stops being true, this component is the thing that has to change — which is the
 * point of writing them down where a user can see them.
 */
export function PrivacyStatement() {
  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>What Halcyon sends</h3>

      <ul className={styles.list}>
        <li>Mail goes to your provider, over TLS. Nothing else leaves this machine.</li>
        <li>No analytics, no usage statistics, no crash uploads, no update pings about you.</li>
        <li>Your passwords are held by Windows Credential Manager, not by this app.</li>
        <li>
          Your mail is stored unencrypted on this disk, readable by anything that can read your user
          profile.
        </li>
      </ul>

      <p className={styles.hint}>
        The last line is a limitation rather than a choice we are pleased with. Full-disk encryption
        — BitLocker on Windows 11 — is what protects the database at rest, and it is worth checking
        it is on.
      </p>
    </section>
  )
}
