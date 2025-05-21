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
        cookies = load_and_fix_cookies("../upwork_cookies.json")
        context.add_cookies(cookies)

        page = context.new_page()
        page.goto("https://www.upwork.com/nx/search/jobs/?nbs=1&q=qt&sort=recency", timeout=25000)
        page.wait_for_timeout(2000)  # wait for JS content to load
        html = page.content()

        with open("upwork_loggedin.html", "w", encoding="utf-8") as f:
            f.write(html)

        browser.close()
        print("✅ Scraped Upwork and saved HTML.")
        return html

# Parse HTML and extract job data
def parse_jobs(html, existing_links):
    soup = BeautifulSoup(html, "html.parser")
    articles = soup.find_all("article")
    new_jobs = []

    for article in articles:
        title_el = article.find("h4") or article.find("h3") or article.find("h2")
        title = title_el.get_text(strip=True) if title_el else "No title"

        paragraph = article.find("p")
        content = paragraph.get_text(strip=True) if paragraph else "No description"

        link_el = article.find("a", href=True)
        link = "https://www.upwork.com" + link_el["href"] if link_el and link_el["href"].startswith("/") else ""

        if link and link not in existing_links:
            new_jobs.append({
                "title": title,
                "content": content,
                "link": link
            })

    print(f"🆕 Found {len(new_jobs)} new jobs.")
    return new_jobs

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


if __name__ == "__main__":
    while True:
        try:
            # Load existing jobs
            try:
                with open("jobs.json", "r", encoding="utf-8") as f:
                    saved_jobs = json.load(f)
            except FileNotFoundError:
                saved_jobs = []

            existing_links = {job["link"] for job in saved_jobs}
            html = scrape_upwork_with_cookies()
            new_jobs = parse_jobs(html, existing_links)

            for job in new_jobs:
                message = f"*{job['title']}*\n{job['content']}\n[View Job]({job['link']})" if job['link'] else f"*{job['title']}*\n{job['content']}"
                send_telegram_message(message)
                time.sleep(2)

            if new_jobs:
                with open("jobs.json", "w", encoding="utf-8") as f:
                    json.dump(new_jobs + saved_jobs, f, indent=2, ensure_ascii=False)

        except Exception as e:
            print(f"❌ Error: {e}")

        print("⏳ Waiting one minute...")
        time.sleep(60)