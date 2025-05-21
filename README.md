Run PowerShell as Administrator, then execute:

powershell

Set-ExecutionPolicy RemoteSigned


Then confirm by typing:

Y

python -m venv venv

.\venv\Scripts\activate

pip install -r requirements.txt 

playwright install