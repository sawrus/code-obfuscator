package main

import "fmt"

func businessHandler(customerName string) {
    fmt.Println("GO:" + customerName)
}

func main() {
    businessHandler("ok")
}
