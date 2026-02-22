"""Antifraud scoring service."""

from dataclasses import dataclass


@dataclass
class FreezeDecision:
    customer_id: str
    antifraud_score: float
    freeze_reason: str


def calculate_antifraud_score(amount: float, geo_mismatch: bool) -> float:
    base = 35.0
    if amount > 2000:
        base += 40.0
    if geo_mismatch:
        base += 25.0
    return min(base, 100.0)


def make_freeze_decision(customer_id: str, amount: float, geo_mismatch: bool) -> FreezeDecision:
    score = calculate_antifraud_score(amount, geo_mismatch)
    reason = "AUTO_FREEZE" if score >= 75.0 else "MONITOR"
    return FreezeDecision(customer_id=customer_id, antifraud_score=score, freeze_reason=reason)


if __name__ == "__main__":
    decision = make_freeze_decision("cust-42", 2900.0, True)
    print(decision)
