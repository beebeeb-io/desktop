import { describe, expect, test } from 'bun:test'
import {
  normalizeFileVersionListResponse,
  restorableFileVersions,
  restoreVersionId,
} from '../src/desktopApi'

describe('desktop version history helpers', () => {
  test('normalizes the server version-list payload newest first', () => {
    const response = normalizeFileVersionListResponse({
      file_id: 'file-123',
      current_version: 3,
      versions: [
        {
          id: 'object-version-3',
          object_version_id: 'object-version-3',
          file_version_id: null,
          source: 'object_version',
          version_number: 3,
          size_bytes: 9000,
          chunk_count: 2,
          chunk_size_bytes: 5000,
          storage_pool_id: 'pool-1',
          created_by: 'user-1',
          uploaded_by: 'user-1',
          restorable: true,
          created_at: '2026-07-25T12:03:00Z',
        },
        {
          id: 'file-version-2',
          object_version_id: null,
          file_version_id: 'file-version-2',
          source: 'file_version',
          version_number: 2,
          size_bytes: 7000,
          chunk_count: 2,
          chunk_size_bytes: null,
          storage_pool_id: 'pool-1',
          created_by: 'user-1',
          uploaded_by: 'user-1',
          restorable: true,
          created_at: '2026-07-25T12:02:00Z',
        },
      ],
    })

    expect(response).toEqual({
      file_id: 'file-123',
      current_version: 3,
      versions: [
        {
          id: 'object-version-3',
          version_number: 3,
          object_version_id: 'object-version-3',
          file_version_id: null,
          source: 'object_version',
          created_at: '2026-07-25T12:03:00Z',
          size_bytes: 9000,
          chunk_count: 2,
          chunk_size_bytes: 5000,
          storage_pool_id: 'pool-1',
          created_by: 'user-1',
          uploaded_by: 'user-1',
          restorable: true,
          is_current: true,
        },
        {
          id: 'file-version-2',
          version_number: 2,
          object_version_id: null,
          file_version_id: 'file-version-2',
          source: 'file_version',
          created_at: '2026-07-25T12:02:00Z',
          size_bytes: 7000,
          chunk_count: 2,
          chunk_size_bytes: null,
          storage_pool_id: 'pool-1',
          created_by: 'user-1',
          uploaded_by: 'user-1',
          restorable: true,
          is_current: false,
        },
      ],
    })
  })

  test('returns only earlier restorable versions and uses the row id for restore', () => {
    const response = normalizeFileVersionListResponse({
      current_version: 3,
      versions: [
        { id: 'current-object', version_number: 3, restorable: true, created_at: '2026-07-25T12:03:00Z' },
        { id: 'older-file-version', file_version_id: 'older-file-version', version_number: 2, restorable: true, created_at: '2026-07-25T12:02:00Z' },
        { id: 'blocked-object', object_version_id: 'blocked-object', version_number: 1, restorable: false, created_at: '2026-07-25T12:01:00Z' },
      ],
    })

    const restorable = restorableFileVersions(response.versions)

    expect(restorable.map((version) => version.id)).toEqual(['older-file-version'])
    expect(restoreVersionId(restorable[0])).toBe('older-file-version')
  })
})
