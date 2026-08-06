def helper():
    return 1


def recurse(value):
    if value:
        return recurse(value - 1)


class Client:
    def send(self):
        return self.retry()

    def retry(self):
        return self.send()


def outer():
    def inner():
        return 1

    return inner()
