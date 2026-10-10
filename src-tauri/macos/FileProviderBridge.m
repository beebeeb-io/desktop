#import <Foundation/Foundation.h>
#import <FileProvider/FileProvider.h>
#import <dispatch/dispatch.h>
#include <string.h>

static NSString *BeebeebDomainIdentifier = @"io.beebeeb.app.domain";
// Task 1698 part 3 (1696 G8): the domain's display name is "Beebeeb" — the
// app's name and what the system DB + Finder sidebar already show. The old
// "Drive" was the zombie domain's residue (1696 forensics: we registered
// "Drive" while the system showed "Beebeeb"). Which side wins on an existing
// registration is device-verifiable only (addDomain updates the stored
// domain); the registered truth is now the app's own name.
static NSString *BeebeebDomainDisplayName = @"Beebeeb";

static void BeebeebCopyMessage(NSString *message, char *buffer, unsigned long buffer_len) {
    if (buffer == NULL || buffer_len == 0) {
        return;
    }
    const char *utf8 = [message UTF8String];
    strlcpy(buffer, utf8 ?: "unknown File Provider error", buffer_len);
}

static void BeebeebCopyError(NSError *error, char *buffer, unsigned long buffer_len) {
    NSString *message = [NSString stringWithFormat:@"%@ (%@ %ld)",
                                                   error.localizedDescription ?: @"unknown File Provider error",
                                                   error.domain ?: @"unknown-domain",
                                                   (long)error.code];
    BeebeebCopyMessage(message, buffer, buffer_len);
}

static NSFileProviderDomain *BeebeebDomain(void) {
    NSFileProviderDomain *domain = [[NSFileProviderDomain alloc] initWithIdentifier:BeebeebDomainIdentifier
                                                            displayName:BeebeebDomainDisplayName];
    // Task 1698 part 2 (ruling: full trash sync): macOS 13+ surfaces the
    // system trash container as Finder's Trash when this is YES. It IS the
    // default, but it is set explicitly so the ruling is visible in code and
    // survives any future default change.
    if (@available(macOS 13.0, *)) {
        domain.supportsSyncingTrash = YES;
    }
    return domain;
}

static int BeebeebWaitForDomainReady(NSFileProviderDomain *domain,
                                     double timeout_seconds,
                                     char *error_buffer,
                                     unsigned long error_buffer_len) {
    NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:domain];
    if (manager == nil) {
        BeebeebCopyMessage(@"File Provider manager is unavailable for the Beebeeb domain",
                           error_buffer,
                           error_buffer_len);
        return -1;
    }

    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSError *found_error = nil;
    [manager waitForStabilizationWithCompletionHandler:^(NSError *error) {
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];

    int64_t timeout_nanos = (int64_t)(timeout_seconds * (double)NSEC_PER_SEC);
    long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, timeout_nanos));
    if (wait_result != 0) {
        BeebeebCopyMessage(@"Timed out waiting for the Beebeeb File Provider domain to become available",
                           error_buffer,
                           error_buffer_len);
        return -1;
    }
    if (found_error != nil) {
        BeebeebCopyError(found_error, error_buffer, error_buffer_len);
        return -1;
    }
    return 0;
}

// Task 1882 (P0, spec docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md): EVERY
// domain removal keeps the files that never reached the server. The plain
// `removeDomain:completionHandler:` deleted them with the domain (three Finder-created files on a
// device, task 1873). `NSFileProviderDomainRemovalModePreserveDirtyUserData` is macOS 12.0+
// (NSFileProviderManager.h:26, :239; NSFileProviderDefines.h:25); this bridge is built for 14.0
// (src-tauri/build.rs), so there is no fallback to the remove-all form, on purpose.
// `src-tauri/src/finder_removal.rs` pins every removal in the repo to this mode.
//
// Returns: 1 = removed, and the system kept files that had not synced (their folder's path in
// `location_buffer`); 0 = removed, nothing kept; -1 = error (`error_buffer` set).
static int BeebeebRemoveDomainKeepingUnsynced(NSFileProviderDomain *domain,
                                              char *location_buffer,
                                              unsigned long location_buffer_len,
                                              char *error_buffer,
                                              unsigned long error_buffer_len) {
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSURL *found_location = nil;
    __block NSError *found_error = nil;

    [NSFileProviderManager removeDomain:domain
                                   mode:NSFileProviderDomainRemovalModePreserveDirtyUserData
                      completionHandler:^(NSURL *preservedLocation, NSError *error) {
        found_location = preservedLocation;
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

    if (found_error != nil) {
        BeebeebCopyError(found_error, error_buffer, error_buffer_len);
        return -1;
    }
    if (found_location == nil) {
        return 0;
    }
    NSString *path = found_location.path;
    if (path.length == 0) {
        path = found_location.absoluteString;
    }
    BeebeebCopyMessage(path ?: @"", location_buffer, location_buffer_len);
    return 1;
}

static int BeebeebDomainExists(BOOL *exists, char *error_buffer, unsigned long error_buffer_len) {
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSArray<NSFileProviderDomain *> *found_domains = nil;
    __block NSError *found_error = nil;

    [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
        found_domains = domains;
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

    if (found_error != nil) {
        BeebeebCopyError(found_error, error_buffer, error_buffer_len);
        return -1;
    }

    *exists = NO;
    for (NSFileProviderDomain *domain in found_domains) {
        if ([domain.identifier isEqualToString:BeebeebDomainIdentifier]) {
            *exists = YES;
            break;
        }
    }
    return 0;
}

int beebeeb_fp_status(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSArray<NSFileProviderDomain *> *found_domains = nil;
        __block NSError *found_error = nil;

        [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
            found_domains = domains;
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }
        for (NSFileProviderDomain *domain in found_domains) {
            if ([domain.identifier isEqualToString:BeebeebDomainIdentifier]) {
                if (BeebeebWaitForDomainReady(domain, 2.0, error_buffer, error_buffer_len) != 0) {
                    return -1;
                }
                return 1;
            }
        }
        return 0;
    }
}

int beebeeb_fp_visible_url(char *url_buffer, unsigned long url_buffer_len, char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            BeebeebCopyMessage(@"File Provider manager is unavailable for the Beebeeb domain",
                               error_buffer,
                               error_buffer_len);
            return -1;
        }

        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSURL *found_url = nil;
        __block NSError *found_error = nil;
        [manager getUserVisibleURLForItemIdentifier:NSFileProviderRootContainerItemIdentifier
                                  completionHandler:^(NSURL *url, NSError *error) {
            found_url = url;
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];

        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebCopyMessage(@"Timed out resolving the Beebeeb Finder location",
                               error_buffer,
                               error_buffer_len);
            return -1;
        }
        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }
        if (found_url == nil) {
            return 0;
        }
        BeebeebCopyMessage(found_url.path ?: found_url.absoluteString, url_buffer, url_buffer_len);
        return 1;
    }
}

// Issue 4 (task 1524, 2026-09-28): `addDomain` replies success even when the domain
// already exists and the USER has disabled it in System Settings -> General -> Login
// Items & Extensions -> File Providers. A user-disabled domain never launches its
// extension, so the old single monolithic `beebeeb_fp_install` (addDomain, then
// unconditionally `waitForStabilizationWithCompletionHandler`) always burned the full
// 10s timeout and reported a generic, misleading "Timed out waiting..." error in that
// case -- nothing timed out, the user (or macOS) turned the extension off.
//
// `beebeeb_fp_install` is now decomposed into the small primitives below so the
// install decision (wait for stabilization vs. short-circuit with a distinct
// "user disabled" result) is made in Rust, where it is unit-testable as a pure
// function (`macos_file_provider::decide_install_step`) instead of being buried
// inside this Objective-C orchestration. This file now only exposes dumb,
// synchronous-blocking FFI primitives that mirror the existing style of
// `beebeeb_fp_status`/`beebeeb_fp_visible_url`/`beebeeb_fp_remove`.

// Returns: 1 = domain exists, 0 = domain does not exist, -1 = lookup error (error_buffer set).
int beebeeb_fp_domain_exists(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        BOOL exists = NO;
        if (BeebeebDomainExists(&exists, error_buffer, error_buffer_len) != 0) {
            return -1;
        }
        return exists ? 1 : 0;
    }
}

// Returns: 1 = userEnabled == YES, 0 = userEnabled == NO, 2 = domain not registered,
// -1 = lookup error (error_buffer set).
int beebeeb_fp_domain_user_enabled(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSArray<NSFileProviderDomain *> *found_domains = nil;
        __block NSError *found_error = nil;

        [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
            found_domains = domains;
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }

        for (NSFileProviderDomain *domain in found_domains) {
            if ([domain.identifier isEqualToString:BeebeebDomainIdentifier]) {
                // `userEnabled` has shipped since macOS 11.0
                // (FILEPROVIDER_API_AVAILABILITY_V3_IOS == API_AVAILABLE(macos(11.0),
                // ios(16.0)), confirmed against this SDK's FileProvider.framework
                // NSFileProviderDomain.h), well below this bridge's own macOS 14.0
                // minimum deployment target (`-mmacosx-version-min=14.0` in
                // src-tauri/build.rs) -- no `@available` guard is needed to read it here.
                return domain.userEnabled ? 1 : 0;
            }
        }
        return 2;
    }
}

// Calls `+[NSFileProviderManager addDomain:completionHandler:]` only -- does not wait
// for stabilization. Returns: 0 = success, -1 = error (error_buffer set).
int beebeeb_fp_add_domain(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;

        [NSFileProviderManager addDomain:BeebeebDomain() completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }
        return 0;
    }
}

// Waits up to `timeout_seconds` for the Beebeeb domain to stabilize. Returns: 0 =
// success, -1 = timeout or error (error_buffer set, see BeebeebWaitForDomainReady).
int beebeeb_fp_wait_for_domain_ready(double timeout_seconds, char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        return BeebeebWaitForDomainReady(BeebeebDomain(), timeout_seconds, error_buffer, error_buffer_len);
    }
}

// Returns: 1 = removed and files were kept (`location_buffer` = their folder), 0 = removed and
// nothing kept, -1 = error (`error_buffer` set). See BeebeebRemoveDomainKeepingUnsynced.
int beebeeb_fp_remove(char *location_buffer,
                      unsigned long location_buffer_len,
                      char *error_buffer,
                      unsigned long error_buffer_len) {
    @autoreleasepool {
        return BeebeebRemoveDomainKeepingUnsynced(BeebeebDomain(),
                                                  location_buffer,
                                                  location_buffer_len,
                                                  error_buffer,
                                                  error_buffer_len);
    }
}

// Task 1697: signal the replica's WORKING SET after the daemon applied an
// operation batch. Under NSFileProviderReplicatedExtension the working set is
// the ONLY container whose signal the system honors (Mgr.h: "the system will
// ignore any other container"); the extension's enumerator then pulls the
// daemon's change log via ListChanges. Returns: 0 = signaled (or domain not
// registered yet — nothing to signal), -1 = error (error_buffer set).
int beebeeb_fp_signal_working_set(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            // The domain is not (yet) registered: nothing to signal is not a
            // failure — the sync engine must keep running.
            BeebeebCopyMessage(@"File Provider manager is unavailable for the Beebeeb domain",
                               error_buffer,
                               error_buffer_len);
            return 0;
        }
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [manager signalEnumeratorForContainerItemIdentifier:NSFileProviderWorkingSetContainerItemIdentifier
                                          completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        // Bounded wait: signaling is best-effort UI refresh, never sync state.
        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebCopyMessage(@"Timed out signaling the Beebeeb File Provider working set",
                               error_buffer,
                               error_buffer_len);
            return -1;
        }
        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }
        return 0;
    }
}

// Task 1698 part 3 (closes 1696 / audit G8): enumerate EVERY registered
// File Provider domain identifier so the (signed) app can sweep the zombie
// domains left by earlier bundle ids. The identifiers are returned
// newline-separated in `ids_buffer`; the return value is the domain count,
// or -1 on error (error_buffer set). Requires app identity: an unsigned CLI
// context gets -2001 through here as a normal -1 error.
int beebeeb_fp_list_domains(char *ids_buffer, unsigned long ids_buffer_len,
                            char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSArray<NSFileProviderDomain *> *found_domains = nil;
        __block NSError *found_error = nil;

        [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
            found_domains = domains;
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }

        NSMutableString *joined = [NSMutableString string];
        for (NSFileProviderDomain *domain in found_domains) {
            if (joined.length > 0) {
                [joined appendString:@"\n"];
            }
            [joined appendString:domain.identifier ?: @""];
        }
        BeebeebCopyMessage(joined, ids_buffer, ids_buffer_len);
        return (int)found_domains.count;
    }
}

// Task 1698 part 3: remove ONE domain by identifier (the sweep's per-domain
// primitive — the Rust side owns the filtering decision and never passes our
// own identifier here). Task 1882: it keeps un-synced files like every removal.
// Returns: 1 = removed and files were kept (`location_buffer` = their folder),
// 0 = removed (or already gone) and nothing kept, -1 = error (error_buffer set).
int beebeeb_fp_remove_domain_by_id(const char *identifier,
                                   char *location_buffer,
                                   unsigned long location_buffer_len,
                                   char *error_buffer,
                                   unsigned long error_buffer_len) {
    @autoreleasepool {
        if (identifier == NULL) {
            BeebeebCopyMessage(@"no domain identifier given", error_buffer, error_buffer_len);
            return -1;
        }
        NSString *domain_id = [NSString stringWithUTF8String:identifier];
        NSFileProviderDomain *domain = [[NSFileProviderDomain alloc] initWithIdentifier:domain_id
                                                                            displayName:domain_id];
        return BeebeebRemoveDomainKeepingUnsynced(domain,
                                                  location_buffer,
                                                  location_buffer_len,
                                                  error_buffer,
                                                  error_buffer_len);
    }
}
