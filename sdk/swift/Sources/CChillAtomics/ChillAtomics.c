#include "ChillAtomics.h"

#include <stdatomic.h>

static atomic_bool runtime_enabled = false;

bool chill_runtime_is_enabled(void) {
  return atomic_load_explicit(&runtime_enabled, memory_order_acquire);
}

void chill_runtime_set_enabled(bool enabled) {
  atomic_store_explicit(&runtime_enabled, enabled, memory_order_release);
}
