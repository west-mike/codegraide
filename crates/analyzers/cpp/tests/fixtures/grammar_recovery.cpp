#include <string>
#include <type_traits>
#include <utility>

template <typename T> struct can_convert {
    template <typename U>
    static auto test(int)
        -> decltype(std::to_string(std::declval<U>()), std::true_type{});
};

template <typename T,
          typename std::enable_if<std::is_integral<T>::value>::type * = nullptr>
T store(T value) {
    return value;
}

class Parser {
public:
    explicit Parser(std::string name = {}) : name_(std::move(name)) {}

private:
    std::string name_;
};
