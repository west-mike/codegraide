namespace outer {
namespace {
struct Point {
  int x;
  int magnitude() const { return x < 0 ? -x : x; }
};

class Worker {
 public:
  Worker() {}
  ~Worker() {}
  int operator()(int value) { return value + 1; }
};

template <typename T>
T identity(T value) {
  return value;
}

int overloaded(int value) { return value; }
double overloaded(double value) { return value; }

int with_lambda(int value) {
  auto positive = [value]() { return value > 0 ? value : 0; };
  return positive();
}
}  // namespace
}  // namespace outer

int Worker::qualified(int value) { return value; }
