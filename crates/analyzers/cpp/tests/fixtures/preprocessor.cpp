int configured(int value) {
#if FEATURE_ENABLED
  if (value > 0) {
    return value;
  }
#else
  return 0;
#endif
}
