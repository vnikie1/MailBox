/**
 * The word pool the fixture generator draws from.
 *
 * Kept as data rather than as generated lorem ipsum on purpose. A message list is a wall of
 * text at small sizes, and the thing you are actually judging in Phase 2 is how it *reads* —
 * how often a subject wraps, how ragged the right edge of the sender column is, whether a
 * two-line preview usually fills both lines. Lorem gets all of that wrong because its word
 * lengths are not English word lengths, and it never produces the newsletter subject lines
 * and transactional receipts that dominate a real inbox.
 *
 * Everything here is invented. No real person, company or address.
 */

export interface Persona {
  name: string
  address: string
  /** Roughly how often this sender appears, relative to the others. */
  weight: number
}

export const PEOPLE: readonly [Persona, ...Persona[]] = [
  { name: 'Ada Whitfield', address: 'ada.whitfield@northgate.example', weight: 6 },
  { name: 'Marcus Oyelaran', address: 'm.oyelaran@northgate.example', weight: 5 },
  { name: 'Priya Ramanathan', address: 'priya@lantern.example', weight: 5 },
  { name: 'Tomas Bergqvist', address: 'tomas.b@lantern.example', weight: 4 },
  { name: 'Hannah Wexler', address: 'hannah.wexler@driftwood.example', weight: 4 },
  { name: 'Kenji Nakamura', address: 'kenji@driftwood.example', weight: 3 },
  { name: 'Rosalind Achebe', address: 'r.achebe@meridian.example', weight: 3 },
  { name: 'Danny Feld', address: 'danny@meridian.example', weight: 3 },
  { name: 'Isabel Moreau', address: 'isabel.moreau@quayside.example', weight: 3 },
  { name: 'Sam Okonkwo', address: 'sam.okonkwo@quayside.example', weight: 2 },
  { name: 'Greta Lindqvist', address: 'greta@harborline.example', weight: 2 },
  { name: 'Owen Trelawney', address: 'owen.t@harborline.example', weight: 2 },
  { name: 'Nadia Farouk', address: 'nadia.farouk@sablewood.example', weight: 2 },
  { name: 'Bill Hutchings', address: 'bill@sablewood.example', weight: 1 },
  { name: 'Yuki Tanabe', address: 'yuki.tanabe@copperfield.example', weight: 1 },
]

export const SERVICES: readonly [Persona, ...Persona[]] = [
  { name: 'Northgate Billing', address: 'billing@northgate.example', weight: 4 },
  { name: 'Lantern Weekly', address: 'digest@lantern.example', weight: 4 },
  { name: 'Quayside Bank', address: 'alerts@quayside-bank.example', weight: 4 },
  { name: 'Driftwood Support', address: 'support@driftwood.example', weight: 3 },
  { name: 'The Meridian Review', address: 'newsletter@meridian-review.example', weight: 3 },
  { name: 'Harborline Rail', address: 'tickets@harborline-rail.example', weight: 3 },
  { name: 'Copperfield Books', address: 'orders@copperfield.example', weight: 2 },
  { name: 'Sablewood Energy', address: 'no-reply@sablewood-energy.example', weight: 2 },
  { name: 'Fernbank Clinic', address: 'appointments@fernbank.example', weight: 2 },
  { name: 'Union Street Coffee', address: 'hello@unionstreet.example', weight: 1 },
]

/** Threaded, back-and-forth subjects. These are the ones that grow replies. */
export const CONVERSATION_SUBJECTS: string[] = [
  'Draft agenda for Thursday',
  'Q3 numbers — one more pass',
  'Re: the Fenwick contract',
  'Lunch next week?',
  'Feedback on the onboarding flow',
  'Moving the standup to 10',
  'Warehouse inventory discrepancy',
  'Notes from the site visit',
  'Can you take a look at this?',
  'Revised floor plan attached',
  'Follow-up: pricing tiers',
  'Interview panel for the design role',
  'Budget approval needed by Friday',
  'Conference talk proposal',
  'Handover before I go on leave',
  'The staging server is down again',
  'Customer escalation — Barrow account',
  'Rough cut for review',
  'Venue options for the offsite',
  'Quick question about the API limits',
  'Reworked the second section',
  'Are we still on for tomorrow?',
  'Signed and returned',
  'Two small corrections',
  'Thoughts on the new supplier',
]

/** One-shot, machine-sent subjects. These almost never grow a reply. */
export const TRANSACTIONAL_SUBJECTS: string[] = [
  'Your statement is ready',
  'Payment received — thank you',
  'Order #{{n}} has shipped',
  'Appointment confirmed for {{weekday}}',
  'Your booking reference {{ref}}',
  'Security alert: new sign-in',
  'Invoice {{ref}} is due in 7 days',
  'Weekly digest: 12 stories you missed',
  'Your subscription renews soon',
  'Delivery attempted — reschedule online',
  'Verify your email address',
  'Receipt for your purchase',
  'Monthly usage summary',
  'Password changed successfully',
  'Your parcel is out for delivery',
  'Reminder: annual review due',
  'Service maintenance this weekend',
  'Your reservation is confirmed',
  'Statement of account — {{month}}',
  'New device signed in to your account',
]

/**
 * Preview sentences. Mixed lengths on purpose: a preview that always fills exactly two
 * lines hides the ragged bottom edge that real inboxes have, which is one of the things
 * that makes a mock list look subtly fake.
 */
export const PREVIEW_SENTENCES: string[] = [
  'Just wanted to check in before the meeting so we are not caught out by the numbers again.',
  'I have attached the revised version with the changes we discussed on the call.',
  'Sorry for the slow reply — it has been a week.',
  'Let me know if this works and I will get it booked in.',
  'The short answer is yes, but there are a couple of caveats worth walking through.',
  'Thanks for turning this around so quickly, it is much appreciated.',
  'Can we push this to next week? Something has come up on my end.',
  'Everything looks good from here. One small thing on page four.',
  'This is the third time it has happened this month, so I think it is worth investigating.',
  'No action needed, just keeping you in the loop.',
  'I spoke to them this morning and they are happy to proceed on the original terms.',
  'Adding Priya, who has been running this side of things.',
  'Attaching the notes in case it is useful for the write-up.',
  'Quick one — do we have a final headcount for Thursday?',
  'The invoice has been raised and should reach you by the end of the day.',
  'Following up on this as I have not heard back.',
  'Happy with the direction. Let us pick it up properly next week.',
  'That timing works. I will send an invite across shortly.',
  'A summary of your recent activity and anything that needs your attention.',
  'Your account has been updated. No further action is required at this time.',
]

export const ATTACHMENT_NAMES: { filename: string; mime: string }[] = [
  { filename: 'agenda.pdf', mime: 'application/pdf' },
  { filename: 'Q3-summary.xlsx', mime: 'application/vnd.ms-excel' },
  { filename: 'floor-plan-rev4.pdf', mime: 'application/pdf' },
  { filename: 'contract-signed.pdf', mime: 'application/pdf' },
  { filename: 'site-photos.zip', mime: 'application/zip' },
  { filename: 'notes.txt', mime: 'text/plain' },
  { filename: 'invoice-4471.pdf', mime: 'application/pdf' },
  { filename: 'headshot.jpg', mime: 'image/jpeg' },
  { filename: 'proposal-v2.docx', mime: 'application/msword' },
  { filename: 'screenshot.png', mime: 'image/png' },
]

export const BODY_PARAGRAPHS: string[] = [
  'Thanks for getting back to me so quickly. I have gone through the document and left comments in the margin where I thought the argument needed shoring up — nothing structural, mostly places where a reader who has not been in the room would lose the thread.',
  'The main thing I want to flag is the timeline. Working backwards from the deadline, we need sign-off by the middle of next week at the latest, and that assumes nothing comes back from legal. If it does, we lose the buffer entirely.',
  'On the budget question: the figure in the appendix is the one we agreed in March, not the revised one. I have asked for an updated version and will forward it as soon as it lands.',
  'Happy to talk any of this through if it is easier than going back and forth over email. I am fairly free Thursday afternoon and most of Friday.',
  'One more thing — could you check whether the supplier has confirmed the delivery window? It was still outstanding when I last looked and it affects everything downstream.',
]
