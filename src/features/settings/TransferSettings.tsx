import { useCallback, useEffect, useMemo, useState } from 'react'
import { Download, FolderOpen, Upload } from 'lucide-react'

import { useMailboxes } from '@/app/queries'
import type { ImportSource } from '@/lib/generated/ImportSource'
import type { TransferProgress } from '@/lib/generated/TransferProgress'
import {
  exportPickFolder,
  exportRun,
  importPickFiles,
  importRun,
  importSources,
  onTransferProgress,
  runningInTauri,
} from '@/lib/ipc'
import { Button } from '@/ui'

import styles from './settings.module.css'
import pane from './TransferSettings.module.css'

/**
 * Settings → Advanced → Import and export. docs/06 Phase 11.
 *
 * ## Why it is here rather than in a File menu
 *
 * Mail puts import under File ▸ Import Mailboxes. This app has no menu bar and will not get
 * one: Windows owns the caption strip (see `platform/mod.rs`), and adding an in-window menu
 * bar to get one command would cost a band of chrome across every window for the rest of the
 * app's life. Settings is the only chrome there is, and Advanced is where the machinery lives.
 *
 * ## What it says before it writes
 *
 * That importing twice duplicates. There is no stable identity to match an mbox message
 * against — `Message-ID` is missing from a lot of old mail and forged in some of the rest — so
 * a second import of the same file genuinely does add every message again. Telling the user
 * afterwards would be telling them after they had to fix it.
 */

/** A byte count, for a folder list where the useful signal is "big" versus "small". */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`
  if (bytes < 1024 * 1024) return `${String(Math.round(bytes / 1024))} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

export function TransferSettings() {
  const [sources, setSources] = useState<ImportSource[] | null>(null)
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [progress, setProgress] = useState<TransferProgress | null>(null)
  const [format, setFormat] = useState<'mbox' | 'eml'>('mbox')

  const { data: mailboxes = [] } = useMailboxes()

  useEffect(() => {
    void importSources().then(setSources)
  }, [])

  useEffect(() => {
    let cancelled = false
    let stop: (() => void) | undefined

    void onTransferProgress(setProgress).then((unlisten) => {
      if (cancelled) unlisten()
      else stop = unlisten
    })

    return () => {
      cancelled = true
      stop?.()
    }
  }, [])

  const running = progress !== null && progress.finished === null

  // Every folder across every profile, flattened — the file path is the identity, because two
  // profiles can both have a folder called Inbox.
  const folders = useMemo(
    () =>
      (sources ?? []).flatMap((source) =>
        source.folders.map((folder) => ({ ...folder, profile: source.name })),
      ),
    [sources],
  )

  const toggle = useCallback((file: string) => {
    setChosen((current) => {
      const next = new Set(current)
      if (next.has(file)) next.delete(file)
      else next.add(file)
      return next
    })
  }, [])

  const startImport = () => {
    const requests = folders
      .filter((folder) => chosen.has(folder.file))
      .map((folder) => ({ path: `${folder.profile}/${folder.path}`, file: folder.file }))

    if (requests.length === 0) return
    setProgress({ label: '', done: 0, total: requests.length, messages: 0, finished: null })
    void importRun(requests)
  }

  const startFileImport = () => {
    void importPickFiles().then((files) => {
      if (files.length === 0) return

      const requests = files.map((file) => ({
        // The file's own name is the mailbox name. A loose mbox carries no folder structure —
        // a .pst does, and the core ignores this for one and uses the tree inside the file.
        path:
          file
            .split(/[\\/]/)
            .pop()
            ?.replace(/\.(mbox|pst)$/i, '') ?? 'Imported',
        file,
      }))

      setProgress({ label: '', done: 0, total: requests.length, messages: 0, finished: null })
      void importRun(requests)
    })
  }

  const startExport = () => {
    void exportPickFolder().then((directory) => {
      if (directory === null) return

      const ids = mailboxes.map((mailbox) => mailbox.id)
      if (ids.length === 0) return

      setProgress({ label: '', done: 0, total: ids.length, messages: 0, finished: null })
      void exportRun(ids, format, directory)
    })
  }

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Import and export</h3>

      <h4 className={styles.legend}>Import</h4>

      {!runningInTauri ? (
        <p className={styles.hint}>Importing reads files from this machine, so it needs the app.</p>
      ) : sources === null ? (
        <p className={styles.hint}>Looking for mail from other programs…</p>
      ) : folders.length === 0 ? (
        <p className={styles.hint}>
          No Thunderbird mail was found on this machine. You can still choose files yourself — an
          mbox file, or an Outlook .pst.
        </p>
      ) : (
        <>
          <p className={styles.hint}>
            Found in Thunderbird. Everything you choose is copied into a local account called
            &ldquo;On My PC&rdquo; — your existing accounts are not touched.
          </p>

          <ul className={pane.folders}>
            {folders.map((folder) => (
              <li key={folder.file}>
                <label className={styles.choice}>
                  <input
                    type="checkbox"
                    className={styles.checkbox}
                    checked={chosen.has(folder.file)}
                    disabled={running}
                    onChange={() => {
                      toggle(folder.file)
                    }}
                  />
                  <span className={pane.folderName}>
                    {folder.profile}/{folder.path}
                  </span>
                  <span className={styles.name}>{formatSize(folder.bytes)}</span>
                </label>
              </li>
            ))}
          </ul>
        </>
      )}

      <div className={styles.row}>
        <Button variant="bordered" disabled={running || chosen.size === 0} onClick={startImport}>
          <Download size={16} aria-hidden />
          Import {chosen.size > 0 ? `${String(chosen.size)} folders` : 'selected'}
        </Button>

        <Button variant="bordered" disabled={running} onClick={startFileImport}>
          <FolderOpen size={16} aria-hidden />
          Choose files…
        </Button>
      </div>

      <p className={styles.hint}>
        Importing the same mail twice adds it twice. There is nothing in an mbox file that reliably
        identifies a message, so nothing can tell that it has seen one before.
      </p>

      <p className={styles.hint}>
        <strong>Outlook .pst files can be imported</strong> — choose one with the button above.
        Folders, dates, senders and which messages you had read all come across. Two things do not:
        <strong> attachments are not extracted</strong>, and a few older messages store their text
        in a format that cannot be read here, so those arrive with their subject and sender but no
        body. Both are counted and reported when the import finishes.
      </p>

      <h4 className={styles.legend}>Export</h4>

      <fieldset className={styles.group}>
        <legend className={styles.legend}>Save as</legend>

        <label className={styles.choice}>
          <input
            type="radio"
            name="export-format"
            className={styles.radio}
            checked={format === 'mbox'}
            disabled={running}
            onChange={() => {
              setFormat('mbox')
            }}
          />
          One mbox file per mailbox — for Thunderbird, Apple Mail and most other programs
        </label>

        <label className={styles.choice}>
          <input
            type="radio"
            name="export-format"
            className={styles.radio}
            checked={format === 'eml'}
            disabled={running}
            onChange={() => {
              setFormat('eml')
            }}
          />
          A folder of .eml files — one file per message, readable in Windows and Outlook
        </label>
      </fieldset>

      <div className={styles.row}>
        <Button
          variant="bordered"
          disabled={running || mailboxes.length === 0}
          onClick={startExport}
        >
          <Upload size={16} aria-hidden />
          Export all mail…
        </Button>
      </div>

      <p className={styles.hint}>
        Only messages that have been downloaded can be exported. A message whose contents were never
        fetched is an entry in a list and nothing more, and writing it out would produce a file that
        looked like your mail and was not.
      </p>

      {progress !== null && (
        <p className={pane.progress} aria-live="polite">
          {progress.finished === null
            ? `Working… ${String(progress.done)} of ${String(progress.total)}${
                progress.label === '' ? '' : ` — ${progress.label}`
              }`
            : progress.finished.error !== null
              ? `Finished with a problem: ${progress.finished.error}. ${String(
                  progress.finished.messages,
                )} messages were handled.`
              : `Done. ${String(progress.finished.messages)} messages in ${String(
                  progress.finished.folders,
                )} mailboxes${
                  progress.finished.skipped > 0
                    ? `, ${String(progress.finished.skipped)} skipped`
                    : ''
                }.`}
        </p>
      )}
    </section>
  )
}
