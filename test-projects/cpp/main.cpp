#include <iostream>

void businessHandler(const std::string& customerName) {
    std::cout << "CPP:" << customerName << std::endl;
}

int main() {
    businessHandler("ok");
    return 0;
}
