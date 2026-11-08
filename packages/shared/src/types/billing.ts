// Shared billing types: plans, subscriptions, invoices, and storage usage.
import type { components } from './openapi'

export interface Plan {
  id: string
  name: string
  price_eur: number
  price_yearly_eur: number
  storage_bytes: number
  storage_label: string
  per_seat: boolean
  min_seats: number
  features: string[]
  is_active?: boolean
  sort_order?: number
  badge?: string | null
  /** Pricing v2: false for tiers shown but not yet sellable (e.g. Teams/business). */
  purchasable?: boolean
  /** Pricing v2: true for a "coming soon" tier rendered without a checkout CTA. */
  coming_soon?: boolean
  /** Pricing v2: length of the free trial in days (e.g. 14 for Starter/Basic/Pro). */
  trial_days?: number
  local_updated_at?: string | null
  last_pushed_at?: string | null
}

export interface LastTrial {
  kind: 'no_card' | 'mandated' | 'promo' | 'grant'
  /** RFC 3339: when the trial ended unpaid. */
  ended_at: string
  /** A real payment settled since the trial started. */
  charged: boolean
}

export interface Subscription {
  plan: string
  billing_cycle: string
  seats: number
  region: string
  status: string
  is_mock?: boolean
  created_at: string | null
  current_period_end: string | null
  billing_state?: string | null
  past_due_since?: string | null
  pending_downgrade_plan?: string | null
  downgrade_cooldown_until?: string | null
  storage_grace_deadline?: string | null
  /** Set when status === 'paused' (task 0544). ISO timestamp when billing resumes. */
  pause_until?: string | null
  /**
   * Set when status === 'trialing' (task 0905, 14-day free trial). RFC3339
   * timestamp when the trial ends — drives the "N days left" banner and the
   * convert/add-payment CTA. Null/absent for non-trial subscriptions.
   */
  trial_ends_at?: string | null
  /**
   * One-trial-per-account marker (task 0905/1517), mirrors `users.has_used_trial`.
   * `true` once the account has EVER started a trial (whether it lapsed,
   * converted, or is still running) — the server enforces this server-side on
   * `POST /billing/trial/start` (409 `trial_already_used`), but the client MUST
   * also read it to decide whether to show a trial CTA at all: task 1517 found
   * every "Start trial" / compare-table "Upgrade" CTA blindly attempting a
   * trial first and only discovering ineligibility from the 409, which (a)
   * wastes a round trip and (b) surfaced a raw, unparsed error body when the
   * server's conflict payload didn't unwrap cleanly. Gate the trial CTAs on
   * this field directly — true means go straight to paid checkout, never
   * attempt `startTrial()`.
   */
  has_used_trial?: boolean
  /**
   * The account's most recent trial that ENDED unpaid (task 1757 round 2), or null.
   * `has_used_trial` is a lifetime flag and says nothing about HOW a trial ended; this
   * does: `kind` (no_card | mandated | promo), `ended_at`, and whether a real charge
   * settled since it started. The "nothing was charged" notice is gated on it.
   */
  last_trial?: LastTrial | null
  /**
   * Whether the active billing provider supports subscription pause/resume
   * (task 0924). Always false: the current provider has no native pause.
   * The web cancel flow gates its "Pause instead?" step on this — a
   * subscription can never newly enter the pause card (where
   * pauseSubscription() would 501).
   */
  pause_supported?: boolean
  /**
   * The active mandate's payment method (task 0941). `'directdebit'` = a SEPA
   * mandate, which can only be charged off-session (settles in a few days) — the
   * storage-add-on flow then offers an instant pay-now path alongside the mandate
   * charge. `'creditcard'` (or null) = synchronous, immediate-charge UX. Null for
   * trial/legacy subs without a captured mandate.
   */
  mandate_method?: 'creditcard' | 'directdebit' | null
  /**
   * UPGRADE rate-limit eligibility (task 1057). Upgrades (strictly-higher tier)
   * are capped at `upgrade_limit` per `upgrade_window_days` (default 2/60d) to
   * stop upgrade cycling. The web disables the UPGRADE CTAs from these and shows
   * "Next upgrade available {date}" when `can_upgrade === false`. DOWNGRADES are
   * NEVER gated — they are always allowed and apply at the next renewal — so
   * there is intentionally no downgrade-eligibility field here (the 1056
   * `plan_changes_*` / `can_change_plan` set was removed).
   */
  upgrades_used?: number
  /** `max_upgrades` allowed per window (task 1057). */
  upgrade_limit?: number
  /** Rolling upgrade-window size in days (task 1057). */
  upgrade_window_days?: number
  /**
   * When the next upgrade slot frees: the OLDEST in-window upgrade's
   * `created_at + window_days`, as an RFC3339 string. Populated only when at the
   * limit (`can_upgrade === false`); null when a slot is free now (task 1057).
   */
  upgrade_next_available_at?: string | null
  /** `upgrades_used < upgrade_limit` — the upgrade gate (task 1057). */
  can_upgrade?: boolean
  /**
   * Extra storage purchased via the storage add-on (task 0943). The
   * `GET /billing/subscription` response carries this so the post-checkout
   * confirmation poll can detect an instant-pay STORAGE upgrade (plan stays the
   * same, only this rises) — not just plan/cycle changes. TB count, 0 when none.
   */
  extra_storage_tb?: number
  /**
   * The pinned storage-quantity axis (task 0867). For Business plans it is the
   * provisioned TB count; for flat plans it mirrors `extra_storage_tb`. Optional
   * because the subscription endpoint does not always include it — the poll
   * treats an increase here as confirmation when present.
   */
  storage_tb_quantity?: number
  /**
   * Authoritative billed total in cents — the amount Mollie actually charges on
   * this subscription row (task 0947). The `/billing` summary headline renders
   * the monthly total from THIS, not a client-side WASM recompute (which folded
   * in a hard-coded €10.99/TB that could drift from Mollie). Null for
   * legacy/mock subscriptions that never captured a Mollie amount — the
   * web then falls back to `base_plan_cents + addon_cents`.
   */
  mollie_amount_cents?: number | null
  /**
   * The catalog-authoritative recurring BASE price (cents) for plan + cycle +
   * seats, with NO add-ons folded in (task 0947). Drives the "Pro €54.95/mo"
   * line in the breakdown. 0 for free / plans with no catalog price.
   */
  base_plan_cents?: number
  /**
   * The add-on total (cents) for the cycle — extra storage + extra users — from
   * the server's per-unit prices (task 0947). Rendered as a separate "+ N TB ×
   * €X.XX/mo" line in the breakdown. 0 when no add-ons.
   */
  addon_cents?: number
  /**
   * Per-TB storage add-on price (cents) for this plan, sourced from the server's
   * `storage_addon_price_cents` (task 0947) — NOT a hard-coded literal. The web
   * renders "N TB × €{addon_per_tb_cents/100}/mo" from this. Null for plans with
   * no storage add-on.
   */
  addon_per_tb_cents?: number | null
  /**
   * The plan the account is ENTITLED to right now (server task 1601). `"none"`
   * for a `needs_plan` or `lapsed` account (task 1037). Absent on older servers.
   */
  effective_plan?: string
  /**
   * Account lifecycle state (task 1037 — no free signups). Additive; absent on
   * older servers, which clients treat as `"ok"`.
   *   - `ok`         — normal (entitled, or grandfathered Free).
   *   - `needs_plan` — a new-model account that never started a trial or plan:
   *                    the app sends it to `/choose-plan`. Upload quota is 0.
   *   - `lapsed`     — a trial/plan ended unpaid: read-only (quota 0), data
   *                    deleted at `data_deletion_at` unless the user subscribes.
   */
  account_state?: AccountState
  /**
   * RFC3339 deletion date. Originally `lapsed`-only (task 1037); since server
   * PR #129 (task 1605) it is ALSO populated for a never-paid trial the
   * moment it enters `cancelling` (= `access_until` + the 14-day never-paid
   * retention window), not only once it has actually lapsed. Null for a paid
   * cancelling account unless/until the server has computed a real deletion
   * date for it.
   */
  data_deletion_at?: string | null
  /**
   * Task 1814 — what ended, while `account_state` is `lapsed`: `"gift"` for the end of a
   * free coupon period (never a trial, never asked for a payment), `"trial"` otherwise,
   * null when the account is not lapsed. Additive; absent on older servers (treat as
   * `"trial"`, the only lapse they had).
   */
  lapse_kind?: 'gift' | 'trial' | null
  /**
   * True when a `trialing` row already has a Mollie mandate + subscription, so
   * it is charged automatically at `trial_ends_at` (task 1037). False/absent
   * for legacy no-card trials, which still use `POST /billing/trial/convert`.
   */
  trial_auto_converts?: boolean
  /**
   * Task 2106 (server 2092b). RFC 3339 end of the 3-day hold when the bank asked the
   * customer to confirm a trial's first payment (3-D Secure); null/absent otherwise.
   * `trial_auto_converts` stays true during the hold, so THIS field alone says the
   * trial does not charge by itself: it ends at this instant unless the customer
   * confirms (pay-now).
   */
  confirm_payment_by?: string | null
  /**
   * Why a paid trial mandate did NOT start a trial (task 1037, one trial per
   * payment method): `"payment_method_already_used"` when the card / bank
   * account already started a trial on any account. The account stays
   * `needs_plan` (the iDEAL cent is refunded). Null otherwise; cleared on the
   * next `POST /billing/trial/checkout` and on a successful trial.
   */
  trial_block_reason?: string | null
  /**
   * Task 1605 (server PR #129, `GET /billing/subscription`). Set the instant
   * a never-paid mandated trial is cancelled (`subscriptions.canceled_at`) —
   * uploads and new shares are refused (409 `trial_cancelled_read_only`) from
   * this moment, even though `status` stays `cancelling` and view/download
   * keep working until `access_until`. Null for every other state, including
   * a PAID account's ordinary cancel (which keeps full access, uploads
   * included, until `current_period_end` — no upload block at all).
   */
  uploads_blocked_at?: string | null
  /**
   * Task 1605. For a cancelled never-paid trial, the date view/download stop
   * working (= `trial_ends_at` at the moment of cancel) — the read-only
   * window's end, not a renewal. Mirrors `current_period_end` for this case
   * but is the field to read: it stays correct even where `current_period_end`
   * has server-internal meanings that shift. Null outside this state.
   */
  access_until?: string | null
  /**
   * Task 1605. Non-null only while a `trialing` row (i) already holds a
   * Mollie mandate/subscription (`trial_auto_converts: true`) AND (ii) has
   * never had a successful real charge — the storage quota is capped at this
   * many bytes (25 GB) until the first payment settles, after which the
   * server drops the cap and this reads null on the next fetch. Never set for
   * the legacy no-card trial pattern (that one keeps the full plan quota).
   */
  trial_storage_cap_bytes?: number | null
}

/** `GET /billing/subscription` → `account_state` (task 1037). */
export type AccountState = 'ok' | 'needs_plan' | 'lapsed'

export interface Invoice {
  id: string
  number: string
  date: string
  amount_eur: number
  status: string
  period: string
  url?: string
  pdf_url?: string
}

/**
 * The account's storage usage (`GET /files/usage`), generated from the
 * server's OpenAPI document (`contracts/openapi.json`, `SERVER_REV`).
 */
export type StorageUsage = components['schemas']['StorageUsageResponse']

/**
 * Storage usage summary (`GET /billing/usage`), generated from the server's
 * OpenAPI document.
 *
 * `percentage` is a fraction 0.0–1.0 (spec §16.3 decision 4), never 0–100:
 * a usage bar renders `percentage * 100`. The generated schema carries no
 * other percentage contract, so this note is the only place it is stated.
 */
export type BillingUsage = components['schemas']['UsageResponse']

export interface PaymentMethod {
  type: string
  brand?: string
  last4?: string
  exp_month?: number
  exp_year?: number
  iban_last4?: string
  is_default: boolean
  /**
   * SEPA Direct Debit details (Mollie directdebit mandates, task 0942). Additive,
   * nullable — only present for a SEPA mandate; null/absent for card/legacy.
   *   iban_masked          — pre-masked IBAN, e.g. "NL10 •••• •••• •••• 9438"
   *   mandate_reference    — Mollie mandate ref, e.g. "SD54-2280-8872-1850"
   *   account_holder_name  — mandate account holder, e.g. "T. Test"
   */
  iban_masked?: string | null
  mandate_reference?: string | null
  account_holder_name?: string | null
}

export interface ReferralStats {
  code: string
  referral_count: number
  earned_gb: number
}

export interface ReferralEntry {
  /** Anonymised display, e.g. "a***@example.com" */
  display_email: string
  joined_at: string
  status: 'active' | 'pending'
}
