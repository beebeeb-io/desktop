#import <Foundation/Foundation.h>
#import <FileProvider/FileProvider.h>
#import <dispatch/dispatch.h>
#include <string.h>

static NSString *BeebeebDomainIdentifier = @"io.beebeeb.app.domain";
static NSString *BeebeebDomainDisplayName = @"Drive";

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
    return [[NSFileProviderDomain alloc] initWithIdentifier:BeebeebDomainIdentifier
                                               displayName:BeebeebDomainDisplayName];
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

static int BeebeebRemoveDomain(char *error_buffer, unsigned long error_buffer_len) {
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSError *found_error = nil;

    [NSFileProviderManager removeDomain:BeebeebDomain() completionHandler:^(NSError *error) {
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

int beebeeb_fp_remove(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        return BeebeebRemoveDomain(error_buffer, error_buffer_len);
    }
}
