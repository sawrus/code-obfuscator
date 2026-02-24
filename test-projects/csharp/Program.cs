using System;

public class Program {
    static void BusinessHandler(string customerName) {
        Console.WriteLine("CS:" + customerName);
    }

    public static void Main(string[] args) {
        BusinessHandler("ok");
    }
}
