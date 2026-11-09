// Account settings' shapes (B14): what the panes draw for a product, a device, a passkey, an activity event and a
// checkup factor. ./data.ts fills them from the account commands; the board's fixtures use the same shapes.

export type ProductState = 'This app' | 'Signed in' | 'Not signed in'
export interface ProductRow { name: string; state: ProductState }

export interface DeviceGroup {
  /** One session id per Beebeeb app signed in on the device (a row's Sign out ends all of them). */
  session_ids: string[]
  name: string
  os: string
  apps: string[]
  /** RFC 3339, or null when the server has none. */
  last_active_at: string | null
  country_code: string | null
  is_current: boolean
}

export interface Passkey { id: string; name: string; created_at: string; last_used_at: string | null; prf: boolean }

export interface ActivityEvent { id: string; what: string; at: string; country: string | null; address: string | null }

export interface Factor { key: string; satisfied: boolean }
