-- Per-account signatures. docs/01 §6, docs/06 Phase 7.
--
-- On the account rather than in `setting`, because that is what they are: a signature belongs
-- to an identity, and someone with a work account and a personal one wants different ones
-- without thinking about it. Keyed rows in `setting` would model the same thing less honestly
-- and would not be removed when the account is.
ALTER TABLE account ADD COLUMN signature_html TEXT;

-- Where it goes relative to a quoted reply: `above` or `below`.
--
-- Not a cosmetic preference. "Above" puts the signature immediately under what the user just
-- wrote and before the quoted history, which is what people who reply inline expect; "below"
-- puts it at the very bottom, which is what people who top-post expect. Getting it wrong makes
-- every reply look like a mistake, so it is a stored choice rather than a guess.
--
-- Default `above`, matching Mail.
ALTER TABLE account ADD COLUMN signature_placement TEXT NOT NULL DEFAULT 'above';
