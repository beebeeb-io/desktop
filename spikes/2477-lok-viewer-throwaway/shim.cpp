/* THROWAWAY (task 2477 spike). The thinnest possible C layer over LibreOfficeKit: the official init
 * header does the DLL loading, and every call goes through the version-matched vtable structs, so
 * Rust never guesses a struct offset.
 * C++ on purpose: on Windows the 26.2 init header uses reinterpret_cast, so it does not compile as C. */
#include <cassert>
#include <climits>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#define LOK_USE_UNSTABLE_API
#include <LibreOfficeKit/LibreOfficeKitInit.h>
#include <LibreOfficeKit/LibreOfficeKit.h>

extern "C" {
void *bb_lok_init(const char *install_path, const char *profile_url) {
    return lok_init_2(install_path, profile_url);
}
char *bb_lok_error(void *k) { LibreOfficeKit *p = static_cast<LibreOfficeKit *>(k); return p->pClass->getError(p); }
char *bb_lok_version(void *k) { LibreOfficeKit *p = static_cast<LibreOfficeKit *>(k); return p->pClass->getVersionInfo(p); }
void bb_lok_free(void *k, char *s) { LibreOfficeKit *p = static_cast<LibreOfficeKit *>(k); if (s) p->pClass->freeError(s); }
void bb_lok_destroy(void *k) { LibreOfficeKit *p = static_cast<LibreOfficeKit *>(k); p->pClass->destroy(p); }
void *bb_lok_load(void *k, const char *url, const char *options) {
    LibreOfficeKit *p = static_cast<LibreOfficeKit *>(k);
    return p->pClass->documentLoadWithOptions(p, url, options);
}
int bb_doc_type(void *d) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); return p->pClass->getDocumentType(p); }
int bb_doc_parts(void *d) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); return p->pClass->getParts(p); }
char *bb_doc_page_rects(void *d) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); return p->pClass->getPartPageRectangles(p); }
char *bb_doc_part_name(void *d, int n) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); return p->pClass->getPartName(p, n); }
void bb_doc_set_part(void *d, int n) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); p->pClass->setPart(p, n); }
void bb_doc_size(void *d, long *w, long *h) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); p->pClass->getDocumentSize(p, w, h); }
void bb_doc_init_render(void *d, const char *args) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); p->pClass->initializeForRendering(p, args); }
int bb_doc_tile_mode(void *d) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); return p->pClass->getTileMode(p); }
void bb_doc_paint(void *d, unsigned char *buf, int cw, int ch, int x, int y, int w, int h) {
    LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d);
    p->pClass->paintTile(p, buf, cw, ch, x, y, w, h);
}
void bb_doc_destroy(void *d) { LibreOfficeKitDocument *p = static_cast<LibreOfficeKitDocument *>(d); p->pClass->destroy(p); }
} // extern "C"
