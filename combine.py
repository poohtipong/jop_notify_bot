import json
import time
import requests
from bs4 import BeautifulSoup
from playwright.sync_api import sync_playwright

# Telegram credentials
TELEGRAM_TOKEN = "8162475123:AAFJY98J0xRiMUbi_lf2OxAcuM4p1wYm-6Y"
CHAT_ID = "6811995514"

# Load cookies from file
def load_and_fix_cookies(filepath):
    with open(filepath, "r", encoding="utf-8") as f:
        raw_cookies = json.load(f)
        cleaned_cookies = []
        for cookie in raw_cookies:
            cleaned = {
                "name": cookie["name"],
                "value": cookie["value"],
                "domain": cookie["domain"],
                "path": cookie.get("path", "/"),
                "secure": cookie.get("secure", False),
                "httpOnly": cookie.get("httpOnly", False),
                "sameSite": "Lax"
            }
            if "expirationDate" in cookie:
                cleaned["expires"] = int(cookie["expirationDate"])
            cleaned_cookies.append(cleaned)
        return cleaned_cookies

# Scrape Upwork and return HTML content
def scrape_upwork_with_cookies():
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(
            user_agent="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            viewport={"width": 1280, "height": 800}
        )
        cookies = load_and_fix_cookies("upwork_cookies.json")
        context.add_cookies(cookies)

        page = context.new_page()
        page.goto("https://www.upwork.com/nx/search/jobs/?is_sts_vector_search_result=false&nav_dir=pop&q=a", timeout=25000)
        page.wait_for_timeout(2000)  # wait for JS content to load
        html = page.content()

        with open("upwork_loggedin.html", "w", encoding="utf-8") as f:
            f.write(html)

        browser.close()
        print("✅ Scraped Upwork and saved HTML.")
        return html

# Parse HTML and extract job data
def parse_jobs(html):
    soup = BeautifulSoup(html, "html.parser")
    articles = soup.find_all("article")
    jobs = []
    for article in articles:
        title_el = article.find("h4") or article.find("h3") or article.find("h2")
        title = title_el.get_text(strip=True) if title_el else "No title"

        paragraph = article.find("p")
        content = paragraph.get_text(strip=True) if paragraph else "No description"

        link_el = article.find("a", href=True)
        link = "https://www.upwork.com" + link_el["href"] if link_el and link_el["href"].startswith("/") else ""

        jobs.append({
            "title": title,
            "content": content,
            "link": link
        })

    # Save to JSON
    with open("jobs.json", "w", encoding="utf-8") as f:
        json.dump(jobs, f, indent=2, ensure_ascii=False)

    print(f"✅ Parsed and saved {len(jobs)} jobs.")
    return jobs

# Send job to Telegram
def send_telegram_message(message: str):
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {
        "chat_id": CHAT_ID,
        "text": message,
        "parse_mode": "Markdown"
    }
    response = requests.post(url, data=payload)
    print(f"📤 Sent: {response.status_code} - {message[:50]}...")

# Main
if __name__ == "__main__":
    html = scrape_upwork_with_cookies()
    jobs = parse_jobs(html)
    for job in jobs:
        message = f"*{job['title']}*\n{job['content']}\n[View Job]({job['link']})" if job['link'] else f"*{job['title']}*\n{job['content']}"
        send_telegram_message(message)
        time.sleep(2)