#import <Foundation/Foundation.h>
#import <FileProvider/FileProvider.h>
#import <dispatch/dispatch.h>
#include <string.h>

static NSString *BeebeebDomainIdentifier = @"io.beebeeb.app.domain";
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

int beebeeb_fp_install(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        BOOL existed_before_add = NO;
        if (BeebeebDomainExists(&existed_before_add, error_buffer, error_buffer_len) != 0) {
            return -1;
        }

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
        if (BeebeebWaitForDomainReady(BeebeebDomain(), 10.0, error_buffer, error_buffer_len) != 0) {
            if (!existed_before_add) {
                char setup_error[1024];
                strlcpy(setup_error, error_buffer, sizeof(setup_error));

                char cleanup_error[1024];
                if (BeebeebRemoveDomain(cleanup_error, sizeof(cleanup_error)) != 0) {
                    NSString *combined = [NSString stringWithFormat:@"%s; cleanup failed: %s",
                                                                    setup_error,
                                                                    cleanup_error];
                    BeebeebCopyMessage(combined, error_buffer, error_buffer_len);
                }
            }
            return -1;
        }
        return 0;
    }
}

int beebeeb_fp_remove(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        return BeebeebRemoveDomain(error_buffer, error_buffer_len);
    }
}
