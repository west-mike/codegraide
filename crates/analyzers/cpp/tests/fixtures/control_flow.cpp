int decisions(int value) {
  if (value > 0 && value < 10 || value == 20) {
    for (int i = 0; i < value; ++i) {
      while (value > i) {
        --value;
      }
    }
  } else if (value < 0) {
    do {
      ++value;
    } while (value < 0);
  }
  switch (value) {
    case 0:
    case 1:
      break;
    default:
      break;
  }
  try {
    return value == 2 ? 2 : value;
  } catch (...) {
    return 0;
  }
}
